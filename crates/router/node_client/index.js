const WebSocket = require('ws');
const protobufjs = require('protobufjs')
const uuid = require('uuid');
const { once, EventEmitter } = require('events');

const msgEvent = 'message';

class Codec {
  constructor() {
    return protobufjs.load("../../proto/protos/gsb_api.proto")
      .then((root) => {
        this.gsb_api = root
        return Promise.resolve(this)
      }).catch((err) => {
        throw new Error(err)
      })
  }

  encodePacket(type, payload) {
    try {
      var Packet = this.gsb_api.lookup(`GSB_API.Packet`);
      console.log("Packet type: %j", Packet);
      var payload = Packet.create({
        [type]: payload
      })
      console.log("Sending packet: %s %j", type, payload);
      if (Packet.verify(payload))
        throw new Error(`Error verifying payload for packet ${type}`);
      return Packet.encode(payload).finish()
    } catch (error) {
      console.log("Failed to encode packet: %s", error);
    }
  }

  decodePacket(buffer) {
    try {
      const packet = this.gsb_api.lookup(`GSB_API.Packet`)
      return packet.decode(buffer).toJSON();
    } catch (error) {
      console.log("Failed to decode packet: %s", error);
    }
  }
}

// class Connection extends EventEmitter {
class Connection {
  constructor(wsUrl, messageEmitter) {
    // super();
    return new Codec().then((codec) => {
      this.messageEmitter = messageEmitter;
      this.codec = codec;
      this.ws = new WebSocket(wsUrl);
      this.ws.on('message', (data) => this.onMessage(messageEmitter, codec, data));
      this.ws.on('close', function close() {
        console.log("Close");
      });
      return Promise.resolve(this)
    }).catch((err) => {
      throw new Error(err)
    })
  }

  heartbeat() {
    console.log("heartbeat");
    this.isAlive = true;
  }

  sendMessage(type, msg) {
    try {
      var msg = this.codec.encodePacket(type, msg);
      var header = Buffer.allocUnsafe(4);
      header.writeInt32BE(msg.length);
      var payload = Buffer.concat([header, msg]);
      return this.ws.send(payload);
    } catch (error) {
      console.log("Failed to send message: %s", error);
    }
  }

  onMessage(messageEmitter, codec, data) {
    console.log("Received data: %j", data);
    while (data.length > 4) {
      try {
        const length = data.readInt16BE(2)
        if (data.length != length + 4) {
          console.log("Breaking %d expected %d", data.length, length + 4)
          break
        } else {
          console.log("Data length: %d", length);
        }
        const buf = data.slice(4, length + 4)
        data = data.slice(buf.length + 4)

        var packet = codec.decodePacket(data);
        console.log("Received packet: %j", packet);
        messageEmitter.emit(msgEvent, packet)
      } catch (error) {
        console.log("Failed to handle message event: %s", error);
      }
    }
  }
}

var messageEmitter = new EventEmitter();

new Connection('ws://127.0.0.1:7464', messageEmitter).then((connection) => {
  connection.ws.on('open', async (event) => {
    var instance_id = uuid.v4();
    var hello_msg = {
      name: "echo-client",
      version: "0.0",
      instanceId: instance_id,
    };
    connection.sendMessage("hello", hello_msg);
  });

  once(messageEmitter, msgEvent).then((msg) => {
    console.log("Hello response: %j", msg);
    var instance_id = uuid.v4();
    var payload = new Uint8Array(Array(100).keys());
    var call_request_msg = {
      caller: "js_caller",
      address: "echo/test",
      requestId: `${instance_id}-1`,
      data: payload,
      noReply: false,
    };
    connection.sendMessage("callRequest", call_request_msg);
    return once(messageEmitter, msgEvent);
  }).then((msg) => {
    console.log("Call request response: %j", msg);
  }).catch((error) => {
    console.log("Error: %s", error);
  }).finally(() => {
    console.log("Done");
  });
});