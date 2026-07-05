# Sample Boot Logs

This directory is for boot and serial logs captured during bring-up.

Only small, representative samples should be committed. Large or frequently
regenerated logs are gitignored to avoid bloating the repository.

## Current samples

| File | Source | Notes |
|------|--------|-------|
| [`serial.sample.log`](serial.sample.log) | Windows COM7 capture | Very short serial-open event. |
| [`boot_log.sample.txt`](boot_log.sample.txt) | PowerShell capture | U-Boot `bootm` attempt with truncated output. |

To add a new sample, name it `*.sample.log` or `*.sample.txt` so it is not
ignored. Routine debug dumps can be left untracked in this directory.
