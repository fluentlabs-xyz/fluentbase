use ethereum_types::{Address, Bloom, H256, U256};
use ethers::types::Log;
use rlp::*;

/// EVM log's receipt.
#[derive(Clone, Debug, Default)]
pub struct Receipt {
    pub id: usize,
    pub status: u8,
    pub cumulative_gas_used: u64,
    pub bloom: Bloom,
    pub logs: Vec<Log>,
}

impl Encodable for Receipt {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(4);
        s.append(&self.status);
        s.append(&self.cumulative_gas_used);
        s.append(&self.bloom);
        s.begin_list(self.logs.len());
        for log in self.logs.iter() {
            s.begin_list(3);
            s.append(&log.address);
            s.append_list(&log.topics);
            s.append(&log.data.0);
        }
    }
}
