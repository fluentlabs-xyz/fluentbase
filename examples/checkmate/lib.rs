#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
use alloc::string::String;
use fluentbase_sdk::{codec::SolidityABI, entrypoint, SharedAPI};
use shakmaty::{fen::Fen, san::San, CastlingMode, Chess, FromSetup, Position, Setup};

pub fn is_checkmate(board: String, mv: String) -> bool {
    // parse the FEN string to a Fen object
    let Ok(fen) = Fen::from_ascii(board.as_bytes()) else {
        return false;
    };
    // convert the Fen object to a Setup object
    let setup = Setup::from(fen);
    // convert the Setup object to a Chess object
    let Ok(pos) = Chess::from_setup(setup, CastlingMode::Standard) else {
        return false;
    };
    // parse the move string to a San object
    let Ok(san) = mv.parse::<San>() else {
        return false;
    };
    // convert the San object to a Move object
    let Ok(mv) = san.to_move(&pos) else {
        return false;
    };
    // try to play the move on the chess board and get new position
    let Ok(new_pos) = pos.play(mv) else {
        return false;
    };
    // check if the new position is a checkmate
    new_pos.is_checkmate()
}

pub fn main_entry(sdk: impl SharedAPI) {
    let input = sdk.input();
    let (board, mv) = SolidityABI::<(String, String)>::decode(&input, 0).unwrap_or_else(|_| {
        panic!("malformed input");
    });
    let result = is_checkmate(board, mv);
    if !result {
        panic!("not a checkmate");
    }
}

entrypoint!(main_entry);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::bytes::BytesMut;
    use fluentbase_testing::HostTestingContext;

    #[test]
    #[should_panic(expected = "not a checkmate")]
    fn test_is_checkmate_no_checkmate() {
        let mut input = BytesMut::new();
        SolidityABI::encode(
            &(
                "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR".to_string(),
                "e2e4".to_string(),
            ),
            &mut input,
            0,
        )
        .unwrap();
        let sdk = HostTestingContext::default().with_input(input.to_vec());
        main_entry(sdk.clone());
        assert_eq!(sdk.exit_code(), 0);
    }

    #[test]
    fn test_is_checkmate_is_checkmate() {
        let mut input = BytesMut::new();
        SolidityABI::<(String, String)>::encode(
            &(
                "rnbq1k1r/1p1p3p/5npb/2pQ1p2/p1B1P2P/8/PPP2PP1/RNB1K1NR w KQ - 2 11".to_string(),
                "Qf7".to_string(),
            ),
            &mut input,
            0,
        )
        .unwrap();
        let sdk = HostTestingContext::default().with_input(input.to_vec());
        main_entry(sdk.clone());
        assert_eq!(sdk.exit_code(), 0);
    }

    #[test]
    #[should_panic(expected = "malformed input")]
    fn test_is_board_valid_invalid() {
        let mut input = BytesMut::new();
        SolidityABI::encode(&123u64, &mut input, 0).unwrap();
        let sdk = HostTestingContext::default().with_input(input.to_vec());
        main_entry(sdk.clone());
    }
}
