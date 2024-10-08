## fctools

`fctools` is a highly-modular and extensible SDK that allows development of various types of applications that can leverage microVMs and virtualization in general.
Check out the docs.rs documentation for more details.

Due to constraints of Firecracker itself, this crate is **only usable on Linux**, x86_64 is the most tested CPU architecture but aarch64 should work just as well
according to Firecracker's developers (with RISC-V potentially gaining support in the future). It is recommended to also adhere to the host and guest kernel
[support policy](https://github.com/firecracker-microvm/firecracker/blob/main/docs/kernel-policy.md) set by the Firecracker team.
