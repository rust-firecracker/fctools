## fctools

`fctools` is a highly-modular and extensible SDK that allows development of various types of applications that can leverage microVMs via Amazon's [Firecracker](https://firecracker-microvm.github.io/) virtualization technology. Check out the docs.rs documentation for more details on correct usage of the crate's extensive functionality.

Based on the design decisions of the crate, its MSRV values and the host and guest kernel [support policy](https://github.com/firecracker-microvm/firecracker/blob/main/docs/kernel-policy.md) set by the Firecracker team, this crate only supports **the following recommended configurations** (though using more recent Linux host/guest kernels is discouraged, it is, in most cases, relatively harmless):

| `fctools` | Rust               | Firecracker        | Linux host          | Linux guest         | Host/guest CPUs       |
|-----------|--------------------|--------------------|---------------------|---------------------|-----------------------|
| `0.6.x`   | `1.81.0` and above | `1.7.0` and above  | `5.10.x` or `6.1.x` | `5.10.x` or `6.1.x` | `x86_64`              |
| `0.7.x`   | `1.85.0` and above | `1.13.1` and above | `5.10.x` or `6.1.x` | `5.10.x` or `6.1.x` | `x86_64` or `aarch64` |
