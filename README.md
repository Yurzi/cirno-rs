## cirno-rs, A "smartest" proceess scheduler

Cirno will help you to run tasks and keep your computer away from being "freezed" by your tasks.

## Usage

See `ciron-rs --help` for details.

This `cirno` will send singal to control child process.

`SIGALRM` is used to notify child when the child timeout
`SIGTERM` is used to terminate child when resources are insufficient

Every signal will be send three times, and if `SIGTERM` has been send, the child will be KILL(`SIGKILL`) later

## Examples

```shell
$ cirno-rs -m 4 -s 1 -r 2 -p 1 -t 4 examples.list
```
