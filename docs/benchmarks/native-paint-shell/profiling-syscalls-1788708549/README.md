# Incomplete trace cleanup check

The paint probe emitted its final result, but the test driver did not complete.
The background task reached its 300-second deadline.
The collector retained a DGTL file, and the tracer retained `syscalls.log`.
This is not a passed test or a normal performance measurement.

`strace --help` identifies interrupt mode 3 as the default for `-o FILE PROG`.
That mode blocks fatal signals. The driver sent SIGTERM to the tracer after the paint result.
The tracer did not stop. The final syscall waited approximately 272 seconds until external cleanup.
No owned shell, collector, or tracer process remained after that cleanup.

The corrected driver selects `--interruptible=1` and uses SIGKILL at its deadline and in final cleanup.
Two regression tests use owned shell fixtures without GPU work.
Both passed: cleanup after a result and cleanup after a probe failure.
A fresh real-process run must verify the correction.
