use crate::{Evm, InMemoryDB};
use core::mem::take;
use fluentbase_core::{helpers::calc_create_address, Account};
use fluentbase_genesis::devnet::{devnet_genesis_from_file, KECCAK_HASH_KEY, POSEIDON_HASH_KEY};
use fluentbase_genesis::{Genesis, EXAMPLE_GREETING_ADDRESS};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{address, Address, Bytes, KECCAK_EMPTY, POSEIDON_EMPTY, U256};
use revm_primitives::db::DatabaseCommit;
use revm_primitives::{
    hex, keccak256, AccountInfo, Bytecode, CreateScheme, Env, ExecutionResult, HashMap, TransactTo,
};

#[allow(dead_code)]
struct TestingContext {
    genesis: Genesis,
    db: InMemoryDB,
}

impl Default for TestingContext {
    fn default() -> Self {
        Self::load_from_genesis(devnet_genesis_from_file())
    }
}

#[allow(dead_code)]
impl TestingContext {
    fn load_from_genesis(genesis: Genesis) -> Self {
        // create jzkt and put it into testing context
        let mut db = InMemoryDB::default();
        // convert all accounts from genesis into jzkt
        for (k, v) in genesis.alloc.iter() {
            let poseidon_hash = v
                .storage
                .as_ref()
                .and_then(|v| v.get(&POSEIDON_HASH_KEY).cloned())
                .unwrap_or_else(|| {
                    v.code
                        .as_ref()
                        .map(|v| poseidon_hash(&v).into())
                        .unwrap_or(POSEIDON_EMPTY)
                });
            let keccak_hash = v
                .storage
                .as_ref()
                .and_then(|v| v.get(&KECCAK_HASH_KEY).cloned())
                .unwrap_or_else(|| {
                    v.code
                        .as_ref()
                        .map(|v| keccak256(&v))
                        .unwrap_or(KECCAK_EMPTY)
                });
            let account = Account {
                address: *k,
                balance: v.balance,
                nonce: v.nonce.unwrap_or_default(),
                source_code_size: v.code.as_ref().map(|v| v.len() as u64).unwrap_or_default(),
                source_code_hash: keccak_hash,
                rwasm_code_size: v.code.as_ref().map(|v| v.len() as u64).unwrap_or_default(),
                rwasm_code_hash: poseidon_hash,
            };
            let mut info: AccountInfo = account.into();
            info.code = v.code.clone().map(Bytecode::new_raw);
            info.rwasm_code = v.code.clone().map(Bytecode::new_raw);
            db.insert_account_info(*k, info);
        }
        Self { genesis, db }
    }

    pub(crate) fn get_balance(&mut self, address: Address) -> U256 {
        let account = self.db.load_account(address).unwrap();
        account.info.balance
    }

    pub(crate) fn add_balance(&mut self, address: Address, value: U256) {
        let account = self.db.load_account(address).unwrap();
        account.info.balance += value;
        let mut revm_account = revm_primitives::Account::from(account.info.clone());
        revm_account.mark_touch();
        self.db.commit(HashMap::from([(address, revm_account)]));
    }
}

struct TxBuilder<'a> {
    pub ctx: &'a mut TestingContext,
    pub env: Env,
}

#[allow(dead_code)]
impl<'a> TxBuilder<'a> {
    fn create(ctx: &'a mut TestingContext, deployer: Address, init_code: Bytes) -> Self {
        let mut env = Env::default();
        env.tx.caller = deployer;
        env.tx.transact_to = TransactTo::Create(CreateScheme::Create);
        env.tx.data = init_code;
        env.tx.gas_limit = 300_000_000;
        Self { ctx, env }
    }

    fn call(ctx: &'a mut TestingContext, caller: Address, callee: Address) -> Self {
        let mut env = Env::default();
        env.tx.gas_price = U256::from(1);
        env.tx.caller = caller;
        env.tx.transact_to = TransactTo::Call(callee);
        env.tx.gas_limit = 10_000_000;
        Self { ctx, env }
    }

    fn input(mut self, input: Bytes) -> Self {
        self.env.tx.data = input;
        self
    }

    fn value(mut self, value: U256) -> Self {
        self.env.tx.value = value;
        self
    }

    fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.env.tx.gas_limit = gas_limit;
        self
    }

    fn gas_price(mut self, gas_price: U256) -> Self {
        self.env.tx.gas_price = gas_price;
        self
    }

    fn exec(&mut self) -> ExecutionResult {
        let mut evm = Evm::builder()
            .with_env(Box::new(take(&mut self.env)))
            .with_db(&mut self.ctx.db)
            .build();
        evm.transact_commit().unwrap()
    }
}

fn deploy_evm_tx(ctx: &mut TestingContext, deployer: Address, init_bytecode: Bytes) -> Address {
    // deploy greeting EVM contract
    let result = TxBuilder::create(ctx, deployer, init_bytecode.into()).exec();
    assert!(result.is_success());
    let contract_address = calc_create_address(&deployer, 0);
    let contract_account = ctx.db.accounts.get(&contract_address).unwrap();
    let source_bytecode = ctx
        .db
        .contracts
        .get(&contract_account.info.code_hash)
        .unwrap()
        .bytes()
        .to_vec();
    assert_eq!(contract_account.info.code_hash, keccak256(&source_bytecode));
    assert!(source_bytecode.len() > 0);
    let rwasm_bytecode = ctx
        .db
        .contracts
        .get(&contract_account.info.rwasm_code_hash)
        .unwrap()
        .bytes()
        .to_vec();
    let is_rwasm = rwasm_bytecode.get(0).cloned().unwrap() == 0xef;
    assert!(is_rwasm);
    contract_address
}

fn call_evm_tx(
    ctx: &mut TestingContext,
    caller: Address,
    callee: Address,
    input: Bytes,
) -> ExecutionResult {
    ctx.add_balance(caller, U256::from(1e18));
    // call greeting EVM contract
    TxBuilder::call(ctx, caller, callee).input(input).exec()
}

#[test]
fn test_genesis_greeting() {
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        EXAMPLE_GREETING_ADDRESS,
        Bytes::default(),
    );
    assert!(result.is_success());
    println!("gas used (call): {}", result.gas_used());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "Hello, World",
        core::str::from_utf8(bytes.as_ref()).unwrap()
    );
}

#[test]
fn test_deploy_greeting() {
    // deploy greeting WASM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = deploy_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        include_bytes!("../../../examples/bin/greeting.wasm").into(),
    );
    // call greeting WASM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::default(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "Hello, World",
        core::str::from_utf8(bytes.as_ref()).unwrap()
    );
}

#[test]
fn test_deploy_keccak256() {
    // deploy greeting WASM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = deploy_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        include_bytes!("../../../examples/bin/keccak256.wasm").into(),
    );
    // call greeting WASM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address,
        "Hello, World".into(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529",
        hex::encode(bytes.as_ref()),
    );
}

#[test]
fn test_deploy_panic() {
    // deploy greeting WASM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = deploy_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        include_bytes!("../../../examples/bin/panic.wasm").into(),
    );
    // call greeting WASM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address,
        Bytes::default(),
    );
    assert!(!result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "panicked at examples/src/panic.rs:4:5: it is panic time",
        core::str::from_utf8(bytes.as_ref()).unwrap()
    );
}

#[test]
fn test_evm_greeting() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address = deploy_evm_tx(&mut ctx, DEPLOYER_ADDRESS, hex!("60806040526105ae806100115f395ff3fe608060405234801561000f575f80fd5b506004361061003f575f3560e01c80633b2e97481461004357806345773e4e1461007357806348b8bcc314610091575b5f80fd5b61005d600480360381019061005891906102e5565b6100af565b60405161006a919061039a565b60405180910390f35b61007b6100dd565b604051610088919061039a565b60405180910390f35b61009961011a565b6040516100a6919061039a565b60405180910390f35b60605f8273ffffffffffffffffffffffffffffffffffffffff163190506100d58161012f565b915050919050565b60606040518060400160405280600b81526020017f48656c6c6f20576f726c64000000000000000000000000000000000000000000815250905090565b60605f4790506101298161012f565b91505090565b60605f8203610175576040518060400160405280600181526020017f30000000000000000000000000000000000000000000000000000000000000008152509050610282565b5f8290505f5b5f82146101a457808061018d906103f0565b915050600a8261019d9190610464565b915061017b565b5f8167ffffffffffffffff8111156101bf576101be610494565b5b6040519080825280601f01601f1916602001820160405280156101f15781602001600182028036833780820191505090505b5090505b5f851461027b578180610207906104c1565b925050600a8561021791906104e8565b60306102239190610518565b60f81b8183815181106102395761023861054b565b5b60200101907effffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff191690815f1a905350600a856102749190610464565b94506101f5565b8093505050505b919050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6102b48261028b565b9050919050565b6102c4816102aa565b81146102ce575f80fd5b50565b5f813590506102df816102bb565b92915050565b5f602082840312156102fa576102f9610287565b5b5f610307848285016102d1565b91505092915050565b5f81519050919050565b5f82825260208201905092915050565b5f5b8381101561034757808201518184015260208101905061032c565b5f8484015250505050565b5f601f19601f8301169050919050565b5f61036c82610310565b610376818561031a565b935061038681856020860161032a565b61038f81610352565b840191505092915050565b5f6020820190508181035f8301526103b28184610362565b905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f819050919050565b5f6103fa826103e7565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff820361042c5761042b6103ba565b5b600182019050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601260045260245ffd5b5f61046e826103e7565b9150610479836103e7565b92508261048957610488610437565b5b828204905092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b5f6104cb826103e7565b91505f82036104dd576104dc6103ba565b5b600182039050919050565b5f6104f2826103e7565b91506104fd836103e7565b92508261050d5761050c610437565b5b828206905092915050565b5f610522826103e7565b915061052d836103e7565b9250828201905080821115610545576105446103ba565b5b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffdfea2646970667358221220feebf5ace29c3c3146cb63bf7ca9009c2005f349075639d267cfbd817adde3e564736f6c63430008180033").into());
    // call greeting EVM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address,
        hex!("45773e4e").into(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    let bytes = &bytes[64..75];
    assert_eq!("Hello World", core::str::from_utf8(bytes.as_ref()).unwrap());
}

///
/// Test storage though constructor
///
/// ```solidity
/// // SPDX-License-Identifier: MIT
/// pragma solidity 0.8.24;
/// contract Storage {
///   uint256 private value;
///   constructor() payable {
///     value = 100;
///   }
///   function getValue() public view returns (uint256) {
///     return value;
///   }
/// }
/// ```
///
#[test]
fn test_evm_storage() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const DEPLOYER_ADDRESS: Address = Address::ZERO;
    let contract_address_1 = deploy_evm_tx(&mut ctx, DEPLOYER_ADDRESS, hex!("608060405260645f8190555060af806100175f395ff3fe6080604052348015600e575f80fd5b50600436106026575f3560e01c80632096525514602a575b5f80fd5b60306044565b604051603b91906062565b60405180910390f35b5f8054905090565b5f819050919050565b605c81604c565b82525050565b5f60208201905060735f8301846055565b9291505056fea26469706673582212206a2e6da07d41af2063301a33093a60613dd63420518670788aa99d7d8f47625564736f6c63430008180033").into());
    let contract_address_2 = deploy_evm_tx(&mut ctx, DEPLOYER_ADDRESS, hex!("608060405260645f8190555060af806100175f395ff3fe6080604052348015600e575f80fd5b50600436106026575f3560e01c80632096525514602a575b5f80fd5b60306044565b604051603b91906062565b60405180910390f35b5f8054905090565b5f819050919050565b605c81604c565b82525050565b5f60208201905060735f8301846055565b9291505056fea26469706673582212206a2e6da07d41af2063301a33093a60613dd63420518670788aa99d7d8f47625564736f6c63430008180033").into());
    // call greeting EVM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address_1,
        hex!("20965255").into(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default();
    assert_eq!(
        "0000000000000000000000000000000000000000000000000000000000000064",
        hex::encode(bytes)
    );
    // call greeting EVM contract
    let result = call_evm_tx(
        &mut ctx,
        DEPLOYER_ADDRESS,
        contract_address_2,
        hex!("20965255").into(),
    );
    assert!(result.is_success());
    let bytes = result.output().unwrap_or_default().iter().as_slice();
    assert_eq!(
        "0000000000000000000000000000000000000000000000000000000000000064",
        hex::encode(bytes)
    );
}

#[test]
fn test_simple_send() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    const RECIPIENT_ADDRESS: Address = address!("1092381297182319023812093812312309123132");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(1e9);
    let result = TxBuilder::call(&mut ctx, SENDER_ADDRESS, RECIPIENT_ADDRESS)
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    assert!(result.is_success());
    let tx_cost = gas_price * U256::from(result.gas_used());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18) - tx_cost);
    assert_eq!(ctx.get_balance(RECIPIENT_ADDRESS), U256::from(1e18));
}

#[test]
fn test_create_send() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(2e9);
    let result = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        include_bytes!("../../../examples/bin/greeting.wasm").into(),
    )
    .gas_price(gas_price)
    .value(U256::from(1e18))
    .exec();
    let contract_address = calc_create_address(&SENDER_ADDRESS, 0);
    assert!(result.is_success());
    let tx_cost = gas_price * U256::from(result.gas_used());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18) - tx_cost);
    assert_eq!(ctx.get_balance(contract_address), U256::from(1e18));
}

#[test]
fn test_evm_revert() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const SENDER_ADDRESS: Address = address!("1231238908230948230948209348203984029834");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(0);
    let result = TxBuilder::create(&mut ctx, SENDER_ADDRESS, hex!("5f5ffd").into())
        .gas_price(gas_price)
        .value(U256::from(1e18))
        .exec();
    let contract_address = calc_create_address(&SENDER_ADDRESS, 0);
    assert!(!result.is_success());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(2e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(0e18));
    // now send success tx
    let result = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        include_bytes!("../../../examples/bin/greeting.wasm").into(),
    )
    .gas_price(gas_price)
    .value(U256::from(1e18))
    .exec();
    let contract_address = calc_create_address(&SENDER_ADDRESS, 0);
    assert!(result.is_success());
    assert_eq!(ctx.get_balance(SENDER_ADDRESS), U256::from(1e18));
    assert_eq!(ctx.get_balance(contract_address), U256::from(1e18));
}

#[test]
fn test_bridge_contract() {
    // deploy greeting EVM contract
    let mut ctx = TestingContext::default();
    const SENDER_ADDRESS: Address = address!("d9b36c6c8bfcc633bb83372db44d80f352cdfe3f");
    ctx.add_balance(SENDER_ADDRESS, U256::from(2e18));
    let gas_price = U256::from(0);
    // now send success tx
    let mut tx_builder = TxBuilder::create(
        &mut ctx,
        SENDER_ADDRESS,
        hex!("60806040523480156200001157600080fd5b506040805160208082018352600080835283519182019093529182529060036200003c8382620000f9565b5060046200004b8282620000f9565b505050620001c5565b634e487b7160e01b600052604160045260246000fd5b600181811c908216806200007f57607f821691505b602082108103620000a057634e487b7160e01b600052602260045260246000fd5b50919050565b601f821115620000f457600081815260208120601f850160051c81016020861015620000cf5750805b601f850160051c820191505b81811015620000f057828155600101620000db565b5050505b505050565b81516001600160401b0381111562000115576200011562000054565b6200012d816200012684546200006a565b84620000a6565b602080601f8311600181146200016557600084156200014c5750858301515b600019600386901b1c1916600185901b178555620000f0565b600085815260208120601f198616915b82811015620001965788860151825594840194600190910190840162000175565b5085821015620001b55787850151600019600388901b60f8161c191681555b5050505050600190811b01905550565b610bab80620001d56000396000f3fe608060405234801561001057600080fd5b50600436106100cf5760003560e01c806370a082311161008c578063a9059cbb11610066578063a9059cbb146101a2578063c820f146146101b5578063dd62ed3e146101c8578063df1f29ee1461020157600080fd5b806370a082311461015e57806395d89b41146101875780639dc29fac1461018f57600080fd5b806306fdde03146100d4578063095ea7b3146100f257806318160ddd1461011557806323b872dd14610127578063313ce5671461013a57806340c10f1914610149575b600080fd5b6100dc610227565b6040516100e991906107a6565b60405180910390f35b610105610100366004610810565b6102b9565b60405190151581526020016100e9565b6002545b6040519081526020016100e9565b61010561013536600461083a565b6102d3565b604051601281526020016100e9565b61015c610157366004610810565b6102f7565b005b61011961016c366004610876565b6001600160a01b031660009081526020819052604090205490565b6100dc610351565b61015c61019d366004610810565b610360565b6101056101b0366004610810565b6103b1565b61015c6101c336600461093b565b6103bf565b6101196101d63660046109d9565b6001600160a01b03918216600090815260016020908152604080832093909416825291909152205490565b600954600854604080516001600160a01b039384168152929091166020830152016100e9565b60606006805461023690610a0c565b80601f016020809104026020016040519081016040528092919081815260200182805461026290610a0c565b80156102af5780601f10610284576101008083540402835291602001916102af565b820191906000526020600020905b81548152906001019060200180831161029257829003601f168201915b5050505050905090565b6000336102c781858561044c565b60019150505b92915050565b6000336102e185828561045e565b6102ec8585856104dc565b506001949350505050565b6007546001600160a01b031633146103435760405162461bcd60e51b815260206004820152600a60248201526937b7363c9037bbb732b960b11b60448201526064015b60405180910390fd5b61034d828261053b565b5050565b60606005805461023690610a0c565b6007546001600160a01b031633146103a75760405162461bcd60e51b815260206004820152600a60248201526937b7363c9037bbb732b960b11b604482015260640161033a565b61034d8282610571565b6000336102c78185856104dc565b6007546001600160a01b0316156103d557600080fd5b600780546001600160a01b0319163317905560056103f38582610a94565b5060066104008682610a94565b50600880546001600160a01b039283166001600160a01b03199091161790556009805460ff909416600160a01b026001600160a81b031990941692909116919091179190911790555050565b61045983838360016105a7565b505050565b6001600160a01b0383811660009081526001602090815260408083209386168352929052205460001981146104d657818110156104c757604051637dc7a0d960e11b81526001600160a01b0384166004820152602481018290526044810183905260640161033a565b6104d6848484840360006105a7565b50505050565b6001600160a01b03831661050657604051634b637e8f60e11b81526000600482015260240161033a565b6001600160a01b0382166105305760405163ec442f0560e01b81526000600482015260240161033a565b61045983838361067c565b6001600160a01b0382166105655760405163ec442f0560e01b81526000600482015260240161033a565b61034d6000838361067c565b6001600160a01b03821661059b57604051634b637e8f60e11b81526000600482015260240161033a565b61034d8260008361067c565b6001600160a01b0384166105d15760405163e602df0560e01b81526000600482015260240161033a565b6001600160a01b0383166105fb57604051634a1406b160e11b81526000600482015260240161033a565b6001600160a01b03808516600090815260016020908152604080832093871683529290522082905580156104d657826001600160a01b0316846001600160a01b03167f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b9258460405161066e91815260200190565b60405180910390a350505050565b6001600160a01b0383166106a757806002600082825461069c9190610b54565b909155506107199050565b6001600160a01b038316600090815260208190526040902054818110156106fa5760405163391434e360e21b81526001600160a01b0385166004820152602481018290526044810183905260640161033a565b6001600160a01b03841660009081526020819052604090209082900390555b6001600160a01b03821661073557600280548290039055610754565b6001600160a01b03821660009081526020819052604090208054820190555b816001600160a01b0316836001600160a01b03167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef8360405161079991815260200190565b60405180910390a3505050565b600060208083528351808285015260005b818110156107d3578581018301518582016040015282016107b7565b506000604082860101526040601f19601f8301168501019250505092915050565b80356001600160a01b038116811461080b57600080fd5b919050565b6000806040838503121561082357600080fd5b61082c836107f4565b946020939093013593505050565b60008060006060848603121561084f57600080fd5b610858846107f4565b9250610866602085016107f4565b9150604084013590509250925092565b60006020828403121561088857600080fd5b610891826107f4565b9392505050565b634e487b7160e01b600052604160045260246000fd5b600082601f8301126108bf57600080fd5b813567ffffffffffffffff808211156108da576108da610898565b604051601f8301601f19908116603f0116810190828211818310171561090257610902610898565b8160405283815286602085880101111561091b57600080fd5b836020870160208301376000602085830101528094505050505092915050565b600080600080600060a0868803121561095357600080fd5b853567ffffffffffffffff8082111561096b57600080fd5b61097789838a016108ae565b9650602088013591508082111561098d57600080fd5b5061099a888289016108ae565b945050604086013560ff811681146109b157600080fd5b92506109bf606087016107f4565b91506109cd608087016107f4565b90509295509295909350565b600080604083850312156109ec57600080fd5b6109f5836107f4565b9150610a03602084016107f4565b90509250929050565b600181811c90821680610a2057607f821691505b602082108103610a4057634e487b7160e01b600052602260045260246000fd5b50919050565b601f82111561045957600081815260208120601f850160051c81016020861015610a6d5750805b601f850160051c820191505b81811015610a8c57828155600101610a79565b505050505050565b815167ffffffffffffffff811115610aae57610aae610898565b610ac281610abc8454610a0c565b84610a46565b602080601f831160018114610af75760008415610adf5750858301515b600019600386901b1c1916600185901b178555610a8c565b600085815260208120601f198616915b82811015610b2657888601518255948401946001909101908401610b07565b5085821015610b445787850151600019600388901b60f8161c191681555b5050505050600190811b01905550565b808201808211156102cd57634e487b7160e01b600052601160045260246000fdfea264697066735822122020392651e573f9944e7a325289c46a3be569262ab593d01ed253c5598ffd5a9464736f6c63430008140033").into(),
    )
        .gas_price(gas_price);
    let exec_result = tx_builder.exec();
    let contract_address = calc_create_address(&SENDER_ADDRESS, 0);
    assert!(tx_builder.ctx.db.accounts.contains_key(&contract_address));
    let account = tx_builder.ctx.db.accounts.get(&contract_address).unwrap();
    assert!(account.info.rwasm_code.is_some());
    assert!(!account.info.rwasm_code_hash.is_zero());
    assert!(exec_result.is_success());
}
