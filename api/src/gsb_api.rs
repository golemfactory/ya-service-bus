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
pub struct CallRequest {
    #[prost(string, tag = "1")]
    pub caller: std::string::String,
    #[prost(string, tag = "2")]
    pub address: std::string::String,
    #[prost(string, tag = "3")]
    pub request_id: std::string::String,
    #[prost(bytes, tag = "4")]
    pub data: std::vec::Vec<u8>,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CallReply {
    #[prost(string, tag = "1")]
    pub request_id: std::string::String,
    #[prost(enumeration = "CallReplyCode", tag = "2")]
    pub code: i32,
    #[prost(enumeration = "CallReplyType", tag = "3")]
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
    RegisterBadRequest = 400,
    /// already registered
    RegisterConflict = 409,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum UnregisterReplyCode {
    UnregisteredOk = 0,
    NotRegistered = 404,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum CallReplyCode {
    CallReplyOk = 0,
    /// eg. duplicate request ID, service not found etc.
    CallReplyBadRequest = 400,
    /// e.g. service did not respond in time
    ServiceFailure = 500,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum CallReplyType {
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
    CallRequest = 4,
    CallReply = 5,
}
