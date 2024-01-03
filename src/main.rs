use clap::Parser;
use rustix::process::{Pid, Signal};
use rustix::process::kill_process;
use std::fmt::Display;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, Duration};
use sysinfo::{System, SystemExt};
use cirno_rs::process::kill_process_tree;


#[derive(Debug)]
struct Task {
    name: String,
    prog: String,
    args: Vec<String>,
    handler: Command,
    child: Option<Child>,
    start_time: SystemTime,
}

impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.prog == other.prog && self.args == other.args
    }
}

impl Display for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut args = String::new();
        for arg in &self.args {
            args.push_str(arg);
            args.push_str(" ");
        }
        write!(f, "{} {} {}", self.name, self.prog, args)
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        let child = self.child.take();
        // kill it
        if let Some(mut child) = child {
            kill_process_tree(Pid::from_child(&child), Signal::Kill).expect("Failed to drop task");
            child.wait().expect("Failed to drop task");
        }
    }
    
}

impl Task {
    fn new(name: &str, cmd: &str) -> Task {
        let mut count = 0;
        let mut prog = String::new();
        let mut args = Vec::new();
        for token in cmd.split_whitespace() {
            if count == 0 {
                prog.push_str(token);
            } else {
                args.push(token.to_string());
            }

            count += 1;
        }

        let mut res = Task {
            name: name.to_string(),
            prog: prog.clone(),
            args: args.clone(),
            handler: Command::new(prog),
            child: None,
            start_time: SystemTime::now(),
        };
        res.handler.args(args);
        res
    }

    fn spawn(&mut self) {
        if self.child.is_some() {
            self.stop().expect("Failed to respawn process");
        }

        let p = match self.handler.spawn() {
            Ok(p) => Some(p),
            Err(e) => {
                println!("Failed to spawn process: {}", e);
                None
            }
        };
        self.start_time = std::time::SystemTime::now();
        self.child = p;
    }

    fn stop(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let p = self.child.take();

        match p {
            Some(mut child) => {
                let stautus = child.try_wait()?;
                match stautus {
                    Some(status) => {
                        return Ok(Some(status));
                    }
                    None => {
                        kill_process(Pid::from_child(&child),Signal::Term)?;
                        // try three more times 
                        for _ in 0..3 {
                            std::thread::sleep(Duration::from_secs(1));
                            match child.try_wait() {
                                Ok(Some(status)) => {
                                    return Ok(Some(status));
                                }
                                Ok(None) => {
                                    kill_process(Pid::from_child(&child),Signal::Term)?;
                                }
                                Err(e) => {
                                    return Err(e);
                                }
                            }
                        }
                        // kill it
                        kill_process_tree(Pid::from_child(&child),Signal::Kill)?;
                        // wait for free
                        return Ok(Some(child.wait()?));
                    }
                }
            }
            None => Ok(None),
        }
    }

    fn try_wait(&mut self, timeout: usize) -> std::io::Result<Option<std::process::ExitStatus>> {
        match &mut self.child {
            Some(child) => {
                let result = child.try_wait();
                match result {
                    Ok(Some(status)) => {
                        return Ok(Some(status));
                    }
                    Ok(None) => {
                        let elapsed = self.start_time.elapsed().unwrap_or(Duration::from_secs(0));
                        if elapsed.as_secs() > timeout as u64 && timeout > 0 {
                            println!("task: {} timeout", self.name);
                            kill_process(Pid::from_child(&child),Signal::Alarm)?;

                            // try ⑨ more times
                            for _ in 0..9 {
                                std::thread::sleep(Duration::from_millis(100));
                                match child.try_wait() {
                                    Ok(Some(status)) => {
                                        return Ok(Some(status));
                                    }
                                    Ok(None) => {
                                        kill_process(Pid::from_child(&child),Signal::Alarm)?;
                                    }
                                    Err(e) => {
                                        return Err(e);
                                    }
                                }
                            }
                            // just return
                            return Ok(None);
                        } else {
                            return Ok(None);
                        }
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            },
            None => Ok(None),
        }
    }

    fn stdout(&mut self, pipe: Stdio) -> &mut Self {
        self.handler.stdout(pipe);
        self
    }

    fn stdout_from_file(&mut self, filename: &Path) -> &mut Self {
        filename
            .parent()
            .map(|p| fs::create_dir_all(p).expect("Failed to create directory"));
        let file = fs::File::create(filename).expect("Failed to create file");
        self.stdout(Stdio::from(file));
        self
    }

    fn args(&mut self, args: Vec<String>) -> &mut Self {
        self.args = args;
        self.handler.args(&self.args);
        self
    }
}

enum CirnoOpinion {
    Health,
    Normal,
    Bad,
}

struct Scheduler {
    todo_tasks: Vec<Task>,
    max_workers: usize,
    runing_tasks: Vec<Task>,
    system: System,
    sleep_duration: usize,
    reserved_mem: usize,
    per_task_mem: usize,
    timeout: usize,
    force_task: usize,
    load_max: f64,
    load_min: f64,
}

impl Scheduler {
    fn new(max_workers: usize) -> Scheduler {
        Scheduler {
            todo_tasks: Vec::new(),
            max_workers,
            runing_tasks: Vec::new(),
            system: System::new(),
            sleep_duration: 10,
            reserved_mem: 6,
            per_task_mem: 3,
            timeout: 7200,
            force_task: 1,
            load_max: 1.0,
            load_min: 0.85,
        }
    }

    fn set_sleep_duration(&mut self, duration: usize) {
        self.sleep_duration = duration;
    }

    fn set_reserved_mem(&mut self, mem: usize) {
        self.reserved_mem = mem;
    }

    fn set_timeout(&mut self, timeout: usize) {
        self.timeout = timeout;
    }

    fn set_per_task_mem(&mut self, mem: usize) {
        self.per_task_mem = mem;
    }

    fn set_force_task(&mut self, force_task: usize) {
        self.force_task = force_task;
    }

    fn set_load_max(&mut self, load_max: f64) {
        self.load_max = load_max;
    }

    fn set_load_min(&mut self, load_min: f64) {
        self.load_min = load_min;
    }

    fn submit(&mut self, task: Task) {
        println!("submiting task: {}", task);
        self.todo_tasks.push(task);
    }

    fn do_it(&mut self) {
        while self.todo_tasks.len() + self.runing_tasks.len() > 0 {
            // check finished or timeout task
            let mut next_runing_tasks = Vec::new();
            for mut task in self.runing_tasks.drain(..) {
                match task.try_wait(self.timeout) {
                    Ok(Some(status)) => {
                        println!("task: {} finished with status: {}", task.name, status);
                    }
                    Ok(None) => {
                        next_runing_tasks.push(task);
                    }
                    Err(e) => {
                        println!("task: {} failed with error: {}", task.name, e);
                    }
                }
            }
            self.runing_tasks = next_runing_tasks;

            // check cirno's opinion
            let opinion = self.cirno_check();
            match opinion {
                CirnoOpinion::Health => {
                    // try to add new task
                    if self.todo_tasks.len() > 0 {
                        let mut task = self.todo_tasks.pop().unwrap();
                        task.stdout_from_file(Path::new(&format!("run/{}.txtlog", task.name)));
                        task.spawn();
                        println!("task: {} started", task);
                        self.runing_tasks.push(task);
                    }
                    // sleep
                    std::thread::sleep(Duration::from_secs(self.sleep_duration as u64));
                }
                CirnoOpinion::Normal => {
                    // just sleep
                    std::thread::sleep(Duration::from_secs(self.sleep_duration as u64));
                }
                CirnoOpinion::Bad => {
                    // try to stop one task and sleep
                    if self.runing_tasks.len() > self.force_task {
                        let mut task = self.runing_tasks.pop().unwrap();
                        println!("task: {} stopped", task.name);
                        task.stop().expect("Failed to stop task");
                        self.todo_tasks.push(task);
                    }
                    std::thread::sleep(Duration::from_secs(self.sleep_duration as u64));
                }
            }

        }
    }

    fn cirno_check(&mut self) -> CirnoOpinion {
        let runing_amount = self.runing_tasks.len();

        if runing_amount > self.max_workers {
            return CirnoOpinion::Bad;
        }

        self.system.refresh_memory();
        self.system.refresh_cpu();
        
        let load = self.system.load_average().one / self.system.cpus().len() as f64;
        let free_mem = (self.system.available_memory() / (1024 * 1024 * 1024))as usize;

        if free_mem < self.reserved_mem || load > self.load_max {
            return CirnoOpinion::Bad;
        }

        if runing_amount == self.max_workers {
            return CirnoOpinion::Normal;
        }

        if free_mem >= (self.reserved_mem + self.per_task_mem) && load <= self.load_min {
            return CirnoOpinion::Health;
        }

        CirnoOpinion::Normal
    }
}

fn init_runtime(dirname: &str) {
    fs::create_dir_all(dirname).expect("Failed to create runtime directory");
}

fn gen_tasks_from_file(filename: &Path) -> Vec<Task> {
    let contents = fs::read_to_string(filename).expect("Failed to read task list");
    let contents = contents.trim();
    if contents.len() == 0 {
        return Vec::new();
    }
    let mut task_list = Vec::new();
    for line in contents.split("\n") {
        let name: &str = line
            .split_whitespace()
            .collect::<Vec<&str>>()
            .last()
            .unwrap();
        let task = Task::new(name, line);
        println!("generate task from: {line}");
        task_list.push(task);
    }

    return task_list;
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CLIArgs {
    input_list: String,
    #[arg(short, long)]
    max_workers: usize,
    #[arg(short, long)]
    force_task: usize,
    #[arg(short, long)]
    sleep_duartion: usize,
    #[arg(short, long)]
    reserved_mem: usize,
    #[arg(short, long)]
    per_task_mem: usize,
    #[arg(short, long)]
    timeout: usize,
    #[arg(long)]
    load_max: Option<f64>,
    #[arg(long)]
    load_min: Option<f64>,
}

fn main() {
    // parse args
    let cli = CLIArgs::parse();
    let input_filename = &cli.input_list;

    // init runtime dir
    init_runtime("run");

    let mut scheduler = Scheduler::new(cli.max_workers);
    scheduler.set_sleep_duration(cli.sleep_duartion);
    scheduler.set_reserved_mem(cli.reserved_mem);
    scheduler.set_per_task_mem(cli.per_task_mem);
    scheduler.set_timeout(cli.timeout);
    scheduler.set_force_task(cli.force_task);
    if let Some(load_max) = cli.load_max {
        scheduler.set_load_max(load_max);
    }
    if let Some(load_min) = cli.load_min {
        scheduler.set_load_min(load_min);
    }

    for one in gen_tasks_from_file(Path::new(input_filename)) {
        scheduler.submit(one);
    }

    scheduler.do_it();
}
