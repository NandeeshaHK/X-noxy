use crate::protocol::Request;
use bytes::Bytes;
use futures_util::Stream;
use reqwest::Client;
use std::error::Error;

pub struct Forwarder {
    client: Client,
    target_base: String,
}

impl Forwarder {
    pub fn new(target_base: String) -> Self {
        Self {
            client: Client::new(),
            target_base,
        }
    }

    pub async fn forward(
        &self,
        req: Request,
    ) -> Result<(u16, impl Stream<Item = reqwest::Result<Bytes>>), Box<dyn Error + Send + Sync>>
    {
        let url = format!("{}{}", self.target_base, req.path);

        let mut builder = self
            .client
            .request(req.method.parse().unwrap_or(reqwest::Method::GET), &url);

        for (k, v) in req.headers {
            builder = builder.header(k, v);
        }

        if let Some(body) = req.body {
            builder = builder.json(&body);
        }

        let response = builder.send().await?;
        let status = response.status().as_u16();
        let stream = response.bytes_stream();

        Ok((status, stream))
    }
}
