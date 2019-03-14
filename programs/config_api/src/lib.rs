pub mod config_instruction;
pub mod config_state;
pub mod config_transaction;

use solana_sdk::pubkey::Pubkey;

const CONFIG_PROGRAM_ID: [u8; 32] = [
    133, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0,
];

pub fn check_id(program_id: &Pubkey) -> bool {
    program_id.as_ref() == CONFIG_PROGRAM_ID
}

pub fn id() -> Pubkey {
    Pubkey::new(&CONFIG_PROGRAM_ID)
}
