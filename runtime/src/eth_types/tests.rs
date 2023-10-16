use super::decode_block::get_block;
use tokio::test;

#[tokio::test]
async fn test_decode_block() {
    _ = get_block().await;
}
