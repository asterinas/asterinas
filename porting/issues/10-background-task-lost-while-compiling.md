# Issue 10: Background compile task reported as lost but still running

## Symptom

A background compile task started over SSH is marked `lost` by the task
manager, but `cargo build` is still running on the board.

## Cause

The local task manager lost its heartbeat to the remote process. The remote
process itself is unaffected and continues to compile.

## Fix

Do not restart the build. Instead, reconnect to the board and check the real
state:

```bash
ssh anjie@192.168.100.2 "ps aux | grep -E 'cargo|rustc' | grep -v grep"
ssh anjie@192.168.100.2 "tail -40 /tmp/aster-build.log"
```

If the processes are still running, wait for them to finish. Only kill them if
you are sure the build is actually stuck (for example, no CPU usage and no log
activity for a very long time).

## Verification

The log file continues to grow and `ps` shows `cargo`/`rustc` processes. When
they exit, the log should end with a successful build or a clear error message.
