#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::string::String;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{function_id, router, Contract},
    SharedAPI,
};
use shakmaty::{fen::Fen, san::San, CastlingMode, Chess, FromSetup, Position, Setup};

#[derive(Contract)]
pub struct CHESS<SDK> {
    sdk: SDK,
}

pub trait ChessAPI {
    fn is_checkmate(&self, board: String, mv: String) -> bool;
    fn is_board_valid(&self, board: String) -> bool;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> ChessAPI for CHESS<SDK> {
    #[function_id("isCheckmate(string,string)")]
    fn is_checkmate(&self, board: String, mv: String) -> bool {
        // Parse the FEN string to a Fen object
        let fen = match Fen::from_ascii(board.as_bytes()) {
            Ok(fen) => fen,
            Err(_) => return false,
        };

        // Convert the Fen object to a Setup object
        let setup = Setup::from(fen);

        // Convert the Setup object to a Chess object
        let pos = match Chess::from_setup(setup, CastlingMode::Standard) {
            Ok(pos) => pos,
            Err(_) => return false,
        };
        // Parse the move string to a San object
        let san = match mv.parse::<San>() {
            Ok(san) => san,
            Err(_) => return false,
        };

        // Convert the San object to a Move object
        let mv = match san.to_move(&pos) {
            Ok(mv) => mv,
            Err(_) => return false,
        };

        // Try to play the move on the chess board and get new position
        let new_pos = match pos.play(&mv) {
            Ok(pos) => pos,
            Err(_) => return false,
        };

        // Check if the new position is a checkmate
        new_pos.is_checkmate()
    }

    #[function_id("isBoardValid(string)")]
    fn is_board_valid(&self, board: String) -> bool {
        // Parse the FEN string to a Fen object
        let fen = match Fen::from_ascii(board.as_bytes()) {
            Ok(fen) => fen,
            Err(_) => return false,
        };

        // Convert the Fen object to a Setup object
        let setup = Setup::from(fen);

        // Check if the board is valid
        match Chess::from_setup(setup, CastlingMode::Standard) {
            Ok(_) => return true,
            Err(_) => return false,
        };
    }
}

impl<SDK: SharedAPI> CHESS<SDK> {
    fn deploy(&self) {}
}

basic_entrypoint!(CHESS);

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::{sol, SolCall, SolType};
    use fluentbase_sdk::{journal::JournalState, runtime::TestingContext};

    sol!(
        function isCheckmate(string memory board, string memory mv) public view returns (bool);
    );

    #[test]
    fn test_input_output() {
        let board = "rnbq1k1r/1p1p3p/5npb/2pQ1p2/p1B1P2P/8/PPP2PP1/RNB1K1NR w KQ - 2 11";
        let mv = "Qf7";

        let fluent_input = IsCheckmateCall::new((board.to_string(), mv.to_string())).encode();

        let sol_input = isCheckmateCall {
            board: board.to_string(),
            mv: mv.to_string(),
        }
        .abi_encode();

        assert_eq!(fluent_input.to_vec(), sol_input);

        let fluent_output = IsCheckmateReturn((true,)).encode();

        let sol_output = <(alloy_sol_types::sol_data::Bool,) as SolType>::abi_encode(&(true,));

        assert_eq!(fluent_output.to_vec(), sol_output);
    }

    #[test]
    fn test_is_checkmate_no_checkmate() {
        let input = IsCheckmateCall::new((
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR".to_string(),
            "e2e4".to_string(),
        ))
        .encode();

        println!("Input: {:?}", hex::encode(&input));

        let sdk = TestingContext::empty().with_input(input);

        let mut chess = CHESS::new(JournalState::empty(sdk.clone()));

        chess.deploy();
        chess.main();

        let encoded_output = &sdk.take_output();
        println!("encoded output: {:?}", hex::encode(&encoded_output));
        let result = IsCheckmateReturn::decode(&encoded_output.as_slice()).unwrap();

        assert_eq!(result.0 .0, false);
    }

    #[test]
    fn test_is_checkmate_is_checkmate() {
        let input = IsCheckmateCall::new((
            "rnbq1k1r/1p1p3p/5npb/2pQ1p2/p1B1P2P/8/PPP2PP1/RNB1K1NR w KQ - 2 11".to_string(),
            "Qf7".to_string(),
        ))
        .encode();

        println!("Input: {:?}", hex::encode(&input));

        let sdk = TestingContext::empty().with_input(input);
        let mut chess = CHESS::new(JournalState::empty(sdk.clone()));

        chess.deploy();
        chess.main();

        let encoded_output = &sdk.take_output();
        println!("encoded output: {:?}", hex::encode(&encoded_output));
        let result = IsCheckmateReturn::decode(&encoded_output.as_slice()).unwrap();

        assert_eq!(result.0 .0, true);
    }

    #[test]
    fn test_is_board_valid_invalid() {
        let input = IsBoardValidCall::new((
            "rrrq1k1r/1p1p3p/5npb/2pQ1p2/p1B1P2P/8/PPP2PP1/RNB1K1NR w KQ - 2 11".to_string(),
        ))
        .encode();

        println!("Input: {:?}", hex::encode(&input));

        let sdk = TestingContext::empty().with_input(input);
        let mut chess = CHESS::new(JournalState::empty(sdk.clone()));

        chess.deploy();
        chess.main();

        let encoded_output = &sdk.take_output();
        println!("encoded output: {:?}", hex::encode(&encoded_output));
        let result = IsBoardValidReturn::decode(&encoded_output.as_slice()).unwrap();

        assert_eq!(result.0 .0, false);
    }

    #[test]
    fn test_is_board_valid_valid() {
        let input = IsBoardValidCall::new((
            "rnbq1k1r/1p1p3p/5npb/2pQ1p2/p1B1P2P/8/PPP2PP1/RNB1K1NR w KQ - 2 11".to_string(),
        ))
        .encode();

        println!("Input: {:?}", hex::encode(&input));

        let sdk = TestingContext::empty().with_input(input);
        let mut chess = CHESS::new(JournalState::empty(sdk.clone()));

        chess.deploy();
        chess.main();

        let encoded_output = &sdk.take_output();
        println!("encoded output: {:?}", hex::encode(&encoded_output));
        let result = IsBoardValidReturn::decode(&encoded_output.as_slice()).unwrap();

        assert_eq!(result.0 .0, true);
    }
}
