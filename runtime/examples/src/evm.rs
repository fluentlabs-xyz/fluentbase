use revm::{
    primitives::{
        AccountInfo,
        BlockEnv,
        Bytecode,
        CfgEnv,
        Env,
        TransactTo,
        TxEnv,
        B160,
        B256,
        KECCAK_EMPTY,
        U256,
    },
    Database,
    EVM,
};

struct TestDb {}

impl Database for TestDb {
    type Error = ();

    fn basic(&mut self, address: B160) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(Some(AccountInfo {
            balance: U256::from(0),
            nonce: 0,
            code_hash: KECCAK_EMPTY,
            code: None,
        }))
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        todo!()
    }

    fn storage(&mut self, address: B160, index: U256) -> Result<U256, Self::Error> {
        todo!()
    }

    fn block_hash(&mut self, number: U256) -> Result<B256, Self::Error> {
        todo!()
    }
}

pub fn evm() {
    let env = Env {
        cfg: CfgEnv::default(),
        block: BlockEnv::default(),
        tx: TxEnv {
            caller: B160::from(100),
            transact_to: TransactTo::Call(B160::from(200)),
            value: U256::from(0),
            ..Default::default()
        },
    };
    let mut evm = EVM::with_env(env);
    let db = TestDb {};
    evm.database(db);
    let result = evm.transact().unwrap();
}
