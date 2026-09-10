use crate::EvmTestingContextWithGenesis;
use fluentbase_sdk::{Bytes, PRECOMPILE_EIP7951};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
use hex_literal::hex;
use revm::{
    interpreter::gas::calculate_initial_tx_gas,
    primitives::{hardfork::SpecId, Address, B256, U256},
};

const EIP7951_VERIFY_GAS: u64 = 6_900;

#[test]
fn eip7951_genesis_route_charges_osaka_gas() {
    let input = Bytes::from_static(&hex!("4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e"));
    let initial_gas =
        calculate_initial_tx_gas(SpecId::PRAGUE, &input, false, 0, 0, 0).initial_total_gas;
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.add_balance(Address::ZERO, U256::from(100_000));

    let result = TxBuilder::call(&mut ctx, PRECOMPILE_EIP7951)
        .caller(Address::ZERO)
        .input(input)
        .gas_limit(100_000)
        .exec();

    assert!(result.is_success(), "execution failed: {result:?}");
    assert_eq!(result.output(), Some(&B256::with_last_byte(1).into()));
    assert_eq!(result.tx_gas_used() - initial_gas, EIP7951_VERIFY_GAS);
}
