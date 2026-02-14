use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Request {
    pub id: String,
    pub method: String,
    #[serde(alias = "url")]
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ResponseMessage {
    #[serde(rename = "stream_chunk")]
    StreamChunk {
        #[serde(rename = "requestId")]
        request_id: String,
        chunk: String,
    },
    #[serde(rename = "stream_end")]
    StreamEnd {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "statusCode")]
        status_code: u16,
    },
    #[serde(rename = "response")]
    Response {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "statusCode")]
        status_code: u16,
        headers: HashMap<String, String>,
        body: Option<serde_json::Value>,
    },
}
