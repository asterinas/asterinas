# Commands

OSDK provides similar subcommands as Cargo,
and these subcommands have simalar meanings
as corresponding Cargo subcommands.

Currently, OSDK supports the following subcommands:

- **new**: Create a new kernel package or library package
- **build**: Compile the project and its dependencies
- **run**: Run the kernel with a VMM
- **test**: Execute kernel mode unit test by starting a VMM
- **debug**: Debug a remote target via GDB
- **profile**: Profile a remote GDB debug target to collect stack traces
- **check**: Analyze the current package and report errors
- **clippy**: Check the current package and catch common mistakes

The **new**, **build**, **run**, **test** and **debug** subcommands
can accept additional options,
while the **check** and **clippy** subcommands can only accept arguments 
that are compatible with the corresponding Cargo subcommands.
