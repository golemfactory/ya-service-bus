#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RegisterRequest {
    #[prost(string, tag = "1")]
    pub service_id: std::string::String,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RegisterReply {
    #[prost(enumeration = "RegisterReplyCode", tag = "1")]
    pub code: i32,
    /// in case of errors
    #[prost(string, tag = "2")]
    pub message: std::string::String,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UnregisterRequest {
    #[prost(string, tag = "1")]
    pub service_id: std::string::String,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UnregisterReply {
    #[prost(enumeration = "UnregisterReplyCode", tag = "1")]
    pub code: i32,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ServiceCallRequest {
    #[prost(string, tag = "1")]
    pub address: std::string::String,
    #[prost(string, tag = "2")]
    pub request_id: std::string::String,
    #[prost(bytes, tag = "3")]
    pub data: std::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CallRequest {
    #[prost(string, tag = "1")]
    pub request_id: std::string::String,
    #[prost(bytes, tag = "2")]
    pub data: std::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CallReply {
    #[prost(string, tag = "1")]
    pub request_id: std::string::String,
    #[prost(enumeration = "ServiceReplyCode", tag = "2")]
    pub code: i32,
    #[prost(enumeration = "ServiceReplyType", tag = "3")]
    pub reply_type: i32,
    #[prost(bytes, tag = "4")]
    pub data: std::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MessageHeader {
    #[prost(enumeration = "MessageType", tag = "1")]
    pub msg_type: i32,
    #[prost(uint32, tag = "2")]
    pub msg_length: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum RegisterReplyCode {
    RegisteredOk = 0,
    /// e.g. invalid name
    BadRequest = 400,
    /// already registered
    Conflict = 409,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum UnregisterReplyCode {
    UnregisteredOk = 0,
    NotRegistered = 404,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum ServiceReplyCode {
    ServiceReplyOk = 0,
    /// e.g. service did not respond in time
    ServiceFailure = 500,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum ServiceReplyType {
    /// a single response or end of stream
    Full = 0,
    /// i.e. a streaming response
    Partial = 1,
}
// Plumbing

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum MessageType {
    RegisterRequest = 0,
    RegisterReply = 1,
    UnregisterRequest = 2,
    UnregisterReply = 3,
    ServiceCallRequest = 4,
    CallRequest = 5,
    CallReply = 6,
}
