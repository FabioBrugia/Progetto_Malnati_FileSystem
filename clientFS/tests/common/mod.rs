use crate::api_client::ApiClient;

pub fn run_with_client<R, F>(base_url: String, op: F) -> R
where
    F: FnOnce(ApiClient) -> R,
{
    let runtime = tokio::runtime::Runtime::new().expect("runtime creation must succeed");
    let client = ApiClient::new(base_url, "test-token".to_string(), runtime.handle().clone())
        .expect("ApiClient::new must succeed");

    runtime.block_on(async move { op(client) })
}
