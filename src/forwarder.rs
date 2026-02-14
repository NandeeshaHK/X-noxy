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
        let client = Client::builder()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .danger_accept_invalid_certs(true) // For local dev often useful, also avoids some TLS overhead checks
            .user_agent("noxy-worker/0.1") // Custom minimalist UA
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
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
            // fastify/express/proxies often add host headers that mismatch the local target
            if k.eq_ignore_ascii_case("host") || k.eq_ignore_ascii_case("origin") {
                continue;
            }
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
