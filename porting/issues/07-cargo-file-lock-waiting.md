# Issue 07: `Blocking waiting for file lock on build directory`

## Symptom

`cargo osdk build` appears to hang and prints:

```text
Blocking waiting for file lock on build directory
```

## Cause

Multiple `cargo` processes are trying to use the same `target` directory
simultaneously. Cargo serializes access with a file lock, so the second process
waits until the first one releases it.

Common triggers:

- A previous build is still running in the background.
- An IDE or file watcher triggered a second build.
- A stale lock was left behind after a crash.

## Fix

1. Wait briefly. Cargo will resume automatically once the lock is released.
2. If it hangs for a long time, find the other cargo process and stop it:

   ```bash
   ps aux | grep -E 'cargo|rustc' | grep -v grep
   kill <pid>
   ```

3. If no cargo process exists but the lock persists, remove the stale lock
   files in `target/`:

   ```bash
   find target -name '*.lock' -delete
   ```

## Verification

Re-run the build. The "Blocking waiting for file lock" message should disappear
and compilation should proceed.
