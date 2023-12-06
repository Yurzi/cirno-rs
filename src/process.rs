use rustix::process::{Pid, Signal, kill_process};

pub fn kill_process_tree(pid: Pid, sig: Signal) -> std::io::Result<()> {
    let mut process_to_kill = Vec::new();
    let mut children = Vec::new();
    let processes = get_processes();

    children.push(pid);
    while let Some(child) = children.pop() {
        process_to_kill.push(child);
        for process in processes.iter() {
            if let Some(ppid) = getppid(process.clone()) {
                if ppid == child {
                    children.push(process.clone());
                }
            }
        }
    }

    for process in process_to_kill {
        kill_process(process, sig)?;
    }

    Ok(())
}

pub fn get_processes() -> Vec<Pid> {
    let mut processes = Vec::new();
    
    let mut proc_dir = std::fs::read_dir("/proc").unwrap();
    
    while let Some(entry) = proc_dir.next() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            let file_name = path.file_name().unwrap().to_str().unwrap();
            if let Ok(pid) = file_name.parse::<i32>() {
                processes.push(Pid::from_raw(pid).unwrap());
            }
        }
    }
    processes
}

pub fn getppid(pid: Pid) -> Option<Pid> {
    let pid = pid.as_raw_nonzero().get();
    let proc_contents = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    let mut proc_contents = proc_contents.trim().split_whitespace();
    let _pid = proc_contents.next()?.parse::<i32>().ok()?;
    let _comm = proc_contents.next()?;
    let _state = proc_contents.next()?;
    let ppid = proc_contents.next()?.parse::<i32>().ok()?;
    Some(Pid::from_raw(ppid)?)
}