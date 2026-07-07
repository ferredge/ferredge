# README

> [!WARNING]
>
> This codebase is not stable yet.
>
> Expect significant API churn, breaking changes, and sharp edges while taking shape.

### Problem statement

This project is planned to be a device and data agregator for edge devices and also serve as protocol bridge across. It plans to work in tandem with major edge device protocols including:-
- mqtt
- http
- modbus
- coap
- bacnet
- opcua
- uart
- gpio
- onif
- canbus

A big limitation of [EdgeX](https://www.edgexfoundry.org/) is that it is modelled completely in microservice architecture with REST based communication (or via message bus). However, most for industrial applications the entire stack is deployed in a single machine (except occasionally the database / message bus). Here the microservices become an additional overhead, and horizontal scalablity is not always necessary. Also devices aren't able to directly communicate with each other in their native protocols.

Additionally, since it contains too many moving parts debugging root cause issues become a pain point. Hence this project to address these shortcomings.

### So monolith?

Yes, this will be initially tageted as a monolithic application. However we understand that some people want to bring their own protocols, and don't want to necessarily depend on this project to get their protocols implemented. Some protocols do happen to be proprietory and would be difficult to maintain here as an open source repo,

The folks at edgex have resolved this issue by providings SDKs for communicating with edgex (or abstracting away the complexities) for [Golang](https://github.com/edgexfoundry/device-sdk-go) and [C](https://github.com/edgexfoundry/device-sdk-c). Here, since this will be a rust based project an official sdk will be provided with necessary boilerplate to get started. Also, we will provide a shared library for use with any other language.

Additionally we may incorporate various OS specific IPC protocols such as Dbus, COM, Pipes, Sockets, etc. But this would be a future goal.

### What was that about direct communication between devices?

We have an abstraction bridge layer that can convert messages from one protocol to another with near zero-copy conversion. This means that if you have a modbus device and an mqtt device, you can directly communicate between them without needing to convert the modbus message to a mqtt message and then back to modbus. This is achieved by having a common intermediate representation of messages that can be easily converted to and from different protocols.

### What about UI?

Incoming as a future goal. Once we nail down the core functionality.

### What about App service sdk?

Not an immediate goal. You can just use the FFIs or as a library to communicate and do whatever you want.

### What about running this directly in embedded devices?

We plan to first nail down core functionality with the use of std lib even though we would be using or at least some form of compatibility with [embassy framework](https://github.com/embassy-rs/embassy). So not guarenteed in initial versions unless you are running certain std lib supported devices viz [ESP32-S3](https://www.espressif.com/en/products/socs/esp32-s3). Even then not everything is guarenteed to work as certain cryptographic libs are not going to be supported in them at hardware level (OpenSSL being a notorious example). We do plan to address some of these by providing feature flags to choose what all features are necessary.

`ferredge-runtime-embassy` implements the core runtime, network (embassy-net), and serial (embedded-io-async) adapter traits on `no_std` + `alloc`, and is exercised on a bare-metal Cortex-M3 under QEMU (see `qemu-tests/`). The protocol crates (HTTP, Modbus, MQTT — including the live MQTT session/listener) build and run against it via their `embassy-runtime` feature; drivers are constructed with `with_stack(...)` since embassy has no ambient executor. The Modbus driver is additionally generic over a `ModbusTransport`, so single-endpoint devices can use a lean transport (`TcpTransport`, `UdpTransport`, or `SerialTransport`) via `with_transport(...)` and carry only the stack they actually need — an RTU-only device needs no network stack at all. Driver APIs are async-first everywhere; the `*_async`-suffixed methods are canonical and the blocking convenience methods (e.g. `MqttDriver::listener_status`) exist only on the std runtime stacks. Note that the embassy adapters enable the `local` feature of `ferredge-core`, which relaxes the `Send`/`Sync` bounds on the adapter traits for embassy's single-executor model — a build using them cannot also use the tokio/async-std adapters, so workspace-wide `--all-features` builds are unsupported and embassy checks run per-package in CI.

The QEMU tests also have a live mode against real host services: `just up` starts a harness (diagslave over a pty-bridged UART for Modbus RTU, plus mosquitto, an HTTP file server, and a Modbus TCP diagslave that the guest reaches over its emulated LAN9118 NIC through QEMU's built-in user-mode network stack), `just test` runs the guest against it (`--features harness`), `just run` runs the self-contained in-memory mode, and `just down` tears the harness down. Everything runs unprivileged — no root, no extra network daemons.
