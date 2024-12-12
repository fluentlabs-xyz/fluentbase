use solana_program::{
    bpf_loader_upgradeable,
    pubkey::{Pubkey, MAX_SEED_LEN},
};

fn call_program_address_common<'a, 'b: 'a>(
    invoke_context: &'a mut InvokeContext<'b>,
    seeds: &[&[u8]],
    program_id: &Pubkey,
    overlap_outputs: bool,
    syscall: BuiltinFunctionRustInterface<'b>,
) -> Result<(Pubkey, u8), Error> {
    const SEEDS_VA: u64 = 0x100000000;
    const PROGRAM_ID_VA: u64 = 0x200000000;
    const ADDRESS_VA: u64 = 0x300000000;
    const BUMP_SEED_VA: u64 = 0x400000000;
    const SEED_VA: u64 = 0x500000000;

    let config = Config::default();
    let mut address = Pubkey::default();
    let mut bump_seed = 0;
    let mut regions = vec![
        MemoryRegion::new_readonly(bytes_of(program_id), PROGRAM_ID_VA),
        MemoryRegion::new_writable(bytes_of_mut(&mut address), ADDRESS_VA),
        MemoryRegion::new_writable(bytes_of_mut(&mut bump_seed), BUMP_SEED_VA),
    ];

    let mut mock_slices = Vec::with_capacity(seeds.len());
    for (i, seed) in seeds.iter().enumerate() {
        let vm_addr = SEED_VA.saturating_add((i as u64).saturating_mul(0x100000000));
        let mock_slice = MockSlice {
            vm_addr,
            len: seed.len(),
        };
        mock_slices.push(mock_slice);
        regions.push(MemoryRegion::new_readonly(bytes_of_slice(seed), vm_addr));
    }
    regions.push(MemoryRegion::new_readonly(
        bytes_of_slice(&mock_slices),
        SEEDS_VA,
    ));
    let mut memory_mapping = MemoryMapping::new(regions, &config, &SBPFVersion::V2).unwrap();

    let result = syscall(
        invoke_context,
        SEEDS_VA,
        seeds.len() as u64,
        PROGRAM_ID_VA,
        ADDRESS_VA,
        if overlap_outputs {
            ADDRESS_VA
        } else {
            BUMP_SEED_VA
        },
        &mut memory_mapping,
    );
    result.map(|_| (address, bump_seed))
}

fn create_program_address(
    invoke_context: &mut InvokeContext,
    seeds: &[&[u8]],
    address: &Pubkey,
) -> Result<Pubkey, Error> {
    let (address, _) = call_program_address_common(
        invoke_context,
        seeds,
        address,
        false,
        SyscallCreateProgramAddress::rust,
    )?;
    Ok(address)
}

#[test]
fn test_create_program_address() {
    // These tests duplicate the direct tests in solana_program::pubkey

    prepare_mockup!(invoke_context, program_id, bpf_loader::id());
    let address = bpf_loader_upgradeable::id();

    let exceeded_seed = &[127; MAX_SEED_LEN + 1];
    assert_matches!(
        create_program_address(&mut invoke_context, &[exceeded_seed], &address),
        Result::Err(error) if error.downcast_ref::<SyscallError>().unwrap() == &SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded)
    );
    assert_matches!(
        create_program_address(
            &mut invoke_context,
            &[b"short_seed", exceeded_seed],
            &address,
        ),
        Result::Err(error) if error.downcast_ref::<SyscallError>().unwrap() == &SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded)
    );
    let max_seed = &[0; MAX_SEED_LEN];
    assert!(create_program_address(&mut invoke_context, &[max_seed], &address).is_ok());
    let exceeded_seeds: &[&[u8]] = &[
        &[1],
        &[2],
        &[3],
        &[4],
        &[5],
        &[6],
        &[7],
        &[8],
        &[9],
        &[10],
        &[11],
        &[12],
        &[13],
        &[14],
        &[15],
        &[16],
    ];
    assert!(create_program_address(&mut invoke_context, exceeded_seeds, &address).is_ok());
    let max_seeds: &[&[u8]] = &[
        &[1],
        &[2],
        &[3],
        &[4],
        &[5],
        &[6],
        &[7],
        &[8],
        &[9],
        &[10],
        &[11],
        &[12],
        &[13],
        &[14],
        &[15],
        &[16],
        &[17],
    ];
    assert_matches!(
        create_program_address(&mut invoke_context, max_seeds, &address),
        Result::Err(error) if error.downcast_ref::<SyscallError>().unwrap() == &SyscallError::BadSeeds(PubkeyError::MaxSeedLengthExceeded)
    );
    assert_eq!(
        create_program_address(&mut invoke_context, &[b"", &[1]], &address).unwrap(),
        "BwqrghZA2htAcqq8dzP1WDAhTXYTYWj7CHxF5j7TDBAe"
            .parse()
            .unwrap(),
    );
    assert_eq!(
        create_program_address(&mut invoke_context, &["☉".as_ref(), &[0]], &address).unwrap(),
        "13yWmRpaTR4r5nAktwLqMpRNr28tnVUZw26rTvPSSB19"
            .parse()
            .unwrap(),
    );
    assert_eq!(
        create_program_address(&mut invoke_context, &[b"Talking", b"Squirrels"], &address).unwrap(),
        "2fnQrngrQT4SeLcdToJAD96phoEjNL2man2kfRLCASVk"
            .parse()
            .unwrap(),
    );
    let public_key = Pubkey::from_str("SeedPubey1111111111111111111111111111111111").unwrap();
    assert_eq!(
        create_program_address(&mut invoke_context, &[public_key.as_ref(), &[1]], &address)
            .unwrap(),
        "976ymqVnfE32QFe6NfGDctSvVa36LWnvYxhU6G2232YL"
            .parse()
            .unwrap(),
    );
    assert_ne!(
        create_program_address(&mut invoke_context, &[b"Talking", b"Squirrels"], &address).unwrap(),
        create_program_address(&mut invoke_context, &[b"Talking"], &address).unwrap(),
    );
    invoke_context.mock_set_remaining(0);
    assert_matches!(
        create_program_address(&mut invoke_context, &[b"", &[1]], &address),
        Result::Err(error) if error.downcast_ref::<InstructionError>().unwrap() == &InstructionError::ComputationalBudgetExceeded
    );
}
