# Asterinas xfstests Filesystem Directories

Each supported filesystem lives in a directory named after the
`XFSTESTS_FS_TYPE` value used by the top-level `Makefile`.

Required files:

- `config/build_config.mk`: Makefile fragment for build-time settings.
- `config/xfstests.config`: xfstests host options file.
- `prepare.sh`: prepares the configured devices and mount points before
  `./check` starts.
- `run_list/block.list`: tests excluded from every run for this filesystem.
- `run_list/short.list`: default quick run list.
- `run_list/full.list`: broader run list for manual validation.

Copy `template/` when adding a new filesystem.
