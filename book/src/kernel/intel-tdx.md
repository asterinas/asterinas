# Intel TDX

Asterinas can serve as a secure guest OS for Intel TDX-protected virtual machines (VMs).
This documentation describes
how Asterinas can be run and tested easily on a TDX-enabled Intel server.

Intel TDX (Trust Domain Extensions) is a Trusted Execution Environment (TEE) technology
that enhances VM security
by creating isolated, hardware-enforced trust domains
with encrypted memory, secure initialization, and attestation mechanisms.

## Why choose Asterinas for Intel TDX

VM TEEs such as Intel TDX deserve a more secure option for its guest OS than Linux.
Linux,
with its inherent memory safety issues and large Trusted Computing Base (TCB),
has long suffered from security vulnerabilities due to memory safety bugs.
Additionally,
when Linux is used as the guest kernel inside a VM TEE,
it must process untrusted inputs
(over 1500 instances in Linux, per Intel's estimation)
from the host (via hypercalls, MMIO, and etc.).
These untrusted inputs create new attack surfaces
that can be exploited through memory safety vulnerabilities,
known as Iago attacks.

Asterinas offers greater memory safety than Linux,
particularly against Iago attacks.
Thanks to its framekernel architecture,
the memory safety of Asterinas relies solely on the Asterinas Framework,
excluding the safe device drivers built on top of the Asterinas Framework
that may handle untrusted inputs from the host.
For more information, see [our talk on OC3'24](https://www.youtube.com/watch?v=3AQ5lpXujGo).

Here is the [official documentation](https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/documentation.html) for Intel TDX features.

## Prepare the Intel TDX environment

Please make sure your server supports Intel TDX.

See [this guide](https://github.com/canonical/tdx/#4-setup-host-os)
or other materials to enable Intel TDX in host OS.

You can use script in [tools/check-tdx-support.sh](https://github.com/asterinas/asterinas/tree/main/tools/check-tdx-support.sh) to check 
whether TDX is enabled for your CPU. Due to asterinas is not supporting 
KVM now, further steps for checking whether TDX is supported for KVM are omitted.

## Build and run Asterinas

1. Download the latest source code.

    ```bash
    git clone https://github.com/asterinas/asterinas
    ```

2. Run a Docker container as the development environment.

    ```bash
    docker run -it --privileged --network=host -v /dev:/dev -v $(pwd)/asterinas:/root/asterinas asterinas/asterinas:0.18.0-20260603
    ```
    
3. Inside the container,
go to the project folder to build and run Asterinas.

    ```bash
    make run_kernel INTEL_TDX=1
    ```

If everything goes well,
Asterinas is now up and running inside a TD.

## Using GDB to debug

A Trust Domain (TD) is debuggable if its `ATTRIBUTES.DEBUG` bit is 1.
In this mode, the host VMM can use Intel TDX module functions
to read and modify TD VCPU state and TD private memory,
which are not accessible when the TD is non-debuggable.

Start Asterinas in a GDB-enabled TD and wait for debugging connection:

```bash
make gdb_server INTEL_TDX=1
```

Behind the scene, this command adds `debug=on` configuration to the QEMU parameters
to enable TD debuggable mode.

The server will listen at the default address specified in `Makefile`,
i.e., a local TCP port `:1234`.

Start a GDB client in another terminal:

```bash
make gdb_client INTEL_TDX=1
```

Note that you must use hardware assisted breakpoints
because KVM is enabled when debugging a TD.
