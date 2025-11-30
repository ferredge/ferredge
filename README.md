# README

### Problem statement

This project is planned to be a device and data agregator for edge devices and also serve as command and control, serving as a rust based alternative to edgex. It plans to work in tandem with major edge device protocols including:-
- mqtt
- http
- modbus
- coap
- bacnet
- opcua
- uart
- gpio
- onif

A big limitation of edgex is that it is modelled completely in microservice architecture with rest based communication (or via message bus). However, most for industrial applications the entire stack is deployed in a single machine (except occasionally the database / message bus). Here the microservices become an additional overhead, and horizontal scalablity is not always necessary. Also devices aren't able to directly communicate with each other in their native protocols.

Additionally, since it contains too many moving parts debugging root cause issues become a pain point. Hence this project to address these shortcomings.

### So monolith?

Yes, this will be initially tageted as a monolithic application. However we understand that some people want to bring their own protocols, and don't want to necessarily depend on this project to get their protocols implemented. Some protocols do happen to be proprietory and would be difficult to maintain here as an open source repo,

The folks at edgex have resolved this issue by providings SDKs for communicating with edgex (or abstracting away the complexities) for [Golang](https://github.com/edgexfoundry/device-sdk-go) and [C](https://github.com/edgexfoundry/device-sdk-c). Here, since this will be a rust based project an official sdk will be provided with necessary boilerplate to get started. Also, we will provide a shared library for use with any other language.

Additionally we may incorporate various OS specific IPC protocols such as Dbus, COM, Pipes, Sockets, etc. But this would be a future goal.

### What was that about direct communication between devices?

We are planning to have devices be able to communicate with other devices so long as the target can be supported. Keep in mind that most protocols would not support back and forth communcation without data loss and complex conversion mechanisms. For example, you can control a modbus device via coap but not vice versa. At least not without serious abstractions.

### What about UI?

Incoming as a future goal. Once we nail down the core functionality.
