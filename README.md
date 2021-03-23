# Golem Service Bus (a.k.a. GSB)

GSB is a message bus allowing Yagna services to communicate with one another.
It consist of two software components: router (`ya-sb-router`) and client crate
(`ya-service-bus`). GSB router is a socket-based message dispatcher. The client
crate provides a high-level API for connecting to the router and allows local
(i.e. within-process, in-memory) routing. GSB supports two distinct ways of
communication: service calls (one-to-one, bidirectional) and broadcasts
(one-to-many, unidirectional).


### Low-level router API

#### Message format
GSB messages are encoded with protobuf. Message types could be found in
[`gsb_api.proto`](crates/proto/protos/gsb_api.proto) file. Each message is prepended with a 64-bit header.
First 4 bytes of the header are interpreted as big-endian singed integer
encoding message type (for mapping see [Packet](https://github.com/golemfactory/ya-service-bus/blob/3b8ff9cd49fb040cdfeee127f42b28e962a0a9f4/crates/proto/protos/gsb_api.proto#L32-L51) enum).
Next 4 bytes of the header are interpreted as big-endian unsigned integer
representing message length.

#### Operations

##### Register
Register a service on the bus. Accepts service name as a parameter.
Registered service can be called by its name by other processes connected to GSB.
Service name is treated as a prefix, e.g. a service registered under `foo` will
also receive calls to `foo/bar` and `foo/baz`.

##### Unregister
Unregister a service from the bus. No longer receive calls.

##### ServiceCall
Call a service registered on the bus and wait for the reply. Every service call
has an ID, called service's address (name), and call data. Reply from the service
will be returned in one or more `CallReply` messages containing call request ID.

##### Subscribe
Subscribe to a broadcast topic in order to receive all messages published for
this topic.

##### Unsubscribe
Unsubscribe from a broadcast topic. No longer receive messages.

##### Broadcast
Broadcast a message to all subscribers of a given topic.

#### Pings and disconnections
Every 60 seconds router checks for idle connections. If a client has not sent
any message for 60 seconds it is pinged. If a client has not sent any message
for 120 seconds it is disconnected. The timeout is configurable via
`GSB_PING_TIMEOUT` environment variable. When a client is disconnected all
registered services and broadcast subscriptions are removed. All pending calls
to a service that got disconnected are answered with `ServiceFailure` reply.
