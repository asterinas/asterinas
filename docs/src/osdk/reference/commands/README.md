# Commands

OSDK provides similar commands as Cargo,
and these commands have simalar meanings
as corresponding Cargo commands.

Currently, OSDK supports the following commands:

- **new**: Create a new kernel package or library package
- **build**: Compile the project and its dependencies
- **run**: Run the kernel with a VMM
- **test**: Execute kernel mode unit test by starting a VMM
- **check**: Analyze the current package and report errors
- **clippy**: Check the current package and catch common mistakes

The **new**, **build**, **run** and **test** commands
can accept additional options,
while the **check** and **clippy** commands cannot.
