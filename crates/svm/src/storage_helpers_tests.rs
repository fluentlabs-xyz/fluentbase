mod tests {
    use crate::{
        storage_helpers::{ContractPubkeyHelper, StorageChunksWriter, VariableLengthDataWriter},
        test_helpers::new_test_sdk,
    };
    use alloc::rc::Rc;
    use solana_pubkey::Pubkey;

    #[test]
    fn test_storage_writer() {
        let mut sdk = new_test_sdk();
        let pubkey = Pubkey::new_unique();
        let contract_pubkey_helper = ContractPubkeyHelper { pubkey: &pubkey };
        let writer = StorageChunksWriter {
            slot_calc: Rc::new(contract_pubkey_helper),
            _phantom: Default::default(),
        };

        let mut out_data = vec![];

        let initial_data = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
        ];
        writer.write_data(&mut sdk, &initial_data);
        writer
            .read_data(&mut sdk, &mut out_data)
            .expect("expected success");
        assert_eq!(initial_data, out_data);

        let initial_data = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44,
        ];
        writer.write_data(&mut sdk, &initial_data);
        out_data.clear();
        writer
            .read_data(&mut sdk, &mut out_data)
            .expect("expected success");
        assert_eq!(initial_data, out_data);

        let initial_data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        writer.write_data(&mut sdk, &initial_data);
        out_data.clear();
        writer
            .read_data(&mut sdk, &mut out_data)
            .expect("expected success");
        assert_eq!(initial_data, out_data);
    }
}
