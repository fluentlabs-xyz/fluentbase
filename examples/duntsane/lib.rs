#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;
extern crate fluentbase_sdk;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use alloy_sol_types::private::Uint as U256;
use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, signature, Contract},
    BlockContextReader,
    SharedAPI,
};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};

/// Represents the four suits in a deck of cards.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum Suit {
    Hearts,
    Diamonds,
    Clubs,
    Spades,
}

/// Represents the value of a card.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum CardValue {
    Ace,
    Number(u8), // 2-10
    Jack,
    Queen,
    King,
}

impl CardValue {
    /// Converts `CardValue` to its baccarat numerical value.
    fn to_baccarat_value(&self) -> u8 {
        match self {
            CardValue::Ace => 1,
            CardValue::Number(n) => *n % 10,
            CardValue::Jack | CardValue::Queen | CardValue::King => 0,
        }
    }

    /// Converts `CardValue` to its string representation.
    fn to_string(&self) -> String {
        match self {
            CardValue::Ace => "Ace".to_string(),
            CardValue::Number(n) => n.to_string(),
            CardValue::Jack => "Jack".to_string(),
            CardValue::Queen => "Queen".to_string(),
            CardValue::King => "King".to_string(),
        }
    }
}

/// Represents a single card with a value and suit.
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Card {
    value: CardValue,
    suit: Suit,
}

impl Suit {
    /// Converts `Suit` to its string representation.
    fn to_string(&self) -> &'static str {
        match self {
            Suit::Hearts => "Hearts",
            Suit::Diamonds => "Diamonds",
            Suit::Clubs => "Clubs",
            Suit::Spades => "Spades",
        }
    }
}

/// The main contract structure implementing the `RouterAPI` trait.
#[derive(Contract)]
struct ROUTER<SDK> {
    sdk: SDK,
}

pub trait RouterAPI {
    fn random(&self) -> u64;
    fn play_baccarat(
        &mut self,
    ) -> (
        String, // winner
        String, // player card 1
        String, // player card 2
        String, // banker card 1
        String, // banker card 2
        String, // player third card
        String, // banker third card
        u64,    // remaining cards
    );
    fn reset_deck(&mut self) -> u64;
}

/// Implementation of the `RouterAPI` trait for `ROUTER`.
#[router(mode = "solidity")]
impl<SDK: SharedAPI> RouterAPI for ROUTER<SDK> {
    #[signature("function random() external view returns (uint256)")]
    fn random(&self) -> u64 {
        let seed = self.sdk.context().block_timestamp();
        let mut small_rng = SmallRng::seed_from_u64(seed);
        small_rng.gen()
    }

    #[signature("function resetDeck() external returns (uint256)")]
    fn reset_deck(&mut self) -> u64 {
        let deck_count_key = U256::from(2);
        let value = U256::from(104);
        self.sdk.write_storage(deck_count_key, value);
        104
    }

    #[signature("function playBaccarat() external view returns (string,string,string,string,string,string,string,uint256)")]
    fn play_baccarat(&mut self) -> (String, String, String, String, String, String, String, u64) {
        let current_seed = self.sdk.context().block_timestamp();
        let last_seed_key = U256::from(2);
        let deck_count_key = U256::from(1);

        // Fixed storage reads with proper conversion
        let last_seed_value = self.sdk.storage(&last_seed_key);
        let last_seed = u64::try_from(last_seed_value.as_limbs()[0]).unwrap_or(0);

        let deck_count_value = self.sdk.storage(&deck_count_key);
        let mut deck_count = u64::try_from(deck_count_value.as_limbs()[0]).unwrap_or(104);

        // Initialize deck
        if deck_count == 0 || deck_count > 104 {
            deck_count = 104; // Reset to full deck
            self.sdk
                .write_storage(deck_count_key, U256::from(deck_count));
        }

        let mut deck = Vec::new();
        for _ in 0..2 {
            for suit in [Suit::Hearts, Suit::Diamonds, Suit::Clubs, Suit::Spades].iter() {
                deck.push(Card {
                    value: CardValue::Ace,
                    suit: *suit,
                });
                for num in 2..=10 {
                    deck.push(Card {
                        value: CardValue::Number(num),
                        suit: *suit,
                    });
                }
                for &value in &[CardValue::Jack, CardValue::Queen, CardValue::King] {
                    deck.push(Card { value, suit: *suit });
                }
            }
        }

        // Remove cards that have been drawn in previous games
        while deck.len() > deck_count as usize {
            let idx = (current_seed as usize + deck.len()) % deck.len();
            deck.remove(idx);
        }

        // Create RNG with combined seeds
        let seed = current_seed.wrapping_add(last_seed);
        let mut rng = SmallRng::seed_from_u64(seed);

        // Store the current seed
        self.sdk
            .write_storage(last_seed_key, U256::from(current_seed));

        // Draw cards
        let player_card1 = {
            let idx = rng.gen_range(0..deck.len());
            deck.remove(idx)
        };
        let banker_card1 = {
            let idx = rng.gen_range(0..deck.len());
            deck.remove(idx)
        };
        let player_card2 = {
            let idx = rng.gen_range(0..deck.len());
            deck.remove(idx)
        };
        let banker_card2 = {
            let idx = rng.gen_range(0..deck.len());
            deck.remove(idx)
        };

        // Prepare card strings
        let p1_str = format!(
            "{} of {}",
            player_card1.value.to_string(),
            player_card1.suit.to_string()
        );
        let p2_str = format!(
            "{} of {}",
            player_card2.value.to_string(),
            player_card2.suit.to_string()
        );
        let b1_str = format!(
            "{} of {}",
            banker_card1.value.to_string(),
            banker_card1.suit.to_string()
        );
        let b2_str = format!(
            "{} of {}",
            banker_card2.value.to_string(),
            banker_card2.suit.to_string()
        );

        let mut player_third_card = String::from("None");
        let mut banker_third_card = String::from("None");

        let mut player_score =
            (player_card1.value.to_baccarat_value() + player_card2.value.to_baccarat_value()) % 10;
        let mut banker_score =
            (banker_card1.value.to_baccarat_value() + banker_card2.value.to_baccarat_value()) % 10;

        // Natural win check
        if player_score >= 8 || banker_score >= 8 {
            let winner = if player_score > banker_score {
                "Player"
            } else if banker_score > player_score {
                "Banker"
            } else {
                "Tie"
            };

            return (
                String::from(winner),
                p1_str,
                p2_str,
                b1_str,
                b2_str,
                player_third_card,
                banker_third_card,
                deck.len() as u64,
            );
        }

        // Player draws third card
        let mut player_third_value = 0;
        if player_score <= 5 {
            let idx = rng.gen_range(0..deck.len());
            let player_card3 = deck.remove(idx);
            player_third_value = player_card3.value.to_baccarat_value();
            player_score = (player_score + player_third_value) % 10;
            player_third_card = format!(
                "{} of {}",
                player_card3.value.to_string(),
                player_card3.suit.to_string()
            );
        }

        // Banker draws third card
        let banker_draws = if player_third_card == "None" {
            banker_score <= 5
        } else {
            match player_third_value {
                2..=3 => banker_score <= 4,
                4..=5 => banker_score <= 5,
                6..=7 => banker_score <= 6,
                8 => banker_score <= 2,
                9 | 0 | 1 => banker_score <= 3,
                _ => false,
            }
        };

        if banker_draws && !deck.is_empty() {
            let idx = rng.gen_range(0..deck.len());
            let banker_card3 = deck.remove(idx);
            banker_score = (banker_score + banker_card3.value.to_baccarat_value()) % 10;
            banker_third_card = format!(
                "{} of {}",
                banker_card3.value.to_string(),
                banker_card3.suit.to_string()
            );
        }

        let winner = if player_score > banker_score {
            "Player"
        } else if banker_score > player_score {
            "Banker"
        } else {
            "Tie"
        };

        // Update deck count in storage before returning
        self.sdk
            .write_storage(deck_count_key, U256::from(deck.len()));

        (
            String::from(winner),
            p1_str,
            p2_str,
            b1_str,
            b2_str,
            player_third_card,
            banker_third_card,
            deck.len() as u64,
        )
    }
}

/// Implementation block for `ROUTER` struct.
impl<SDK: SharedAPI> ROUTER<SDK> {
    fn deploy(&mut self) {
        let deck_count_key = U256::from(2);
        self.sdk.write_storage(deck_count_key, U256::from(104));
    }
}

basic_entrypoint!(ROUTER);
