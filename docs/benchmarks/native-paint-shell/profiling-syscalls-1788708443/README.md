# Incomplete syscall check

This run used `strace -f --kill-on-exit --syscall-limit=250000` on the owned paint process.
Worker-thread syscalls reached the limit before the paint probe completed.
The driver correctly rejected the run because no final paint result was available.
This is not a completed capture or a performance result.

The original trace had 497,304 lines and 63,013,791 bytes, including split syscall lines.
The retained `main-thread.partial.log` contains only process 417773 (13,599 lines).
The oversized all-thread trace and incomplete DGTL file were removed.
The next check omits `-f` because the measured surface call executes on the main thread.
No recorder capacity or application behavior changed.
