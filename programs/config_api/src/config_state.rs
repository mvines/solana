//! Config state

use bincode::{deserialize, serialize_into, serialized_size, ErrorKind};
use log::*;
use serde_derive::{Deserialize, Serialize};
use solana_sdk::native_program::ProgramError;

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Clone)]
pub struct ConfigState {
    // Sequence number that increments on every update, will never decrement
    pub seq: u64,

    // Configuration data
    pub data: Vec<u8>,
}

impl ConfigState {
    pub fn new(data_len: u64) -> Self {
        Self {
            seq: 0,
            data: vec![0; data_len as usize],
        }
    }

    pub fn max_size(data_len: u64) -> u64 {
        serialized_size(&ConfigState::new(data_len)).unwrap()
    }

    pub fn deserialize(input: &[u8]) -> Result<Self, ProgramError> {
        deserialize(input).map_err(|_| ProgramError::InvalidAccountData)
    }

    pub fn serialize(&self, output: &mut [u8]) -> Result<(), ProgramError> {
        serialize_into(output, self).map_err(|err| match *err {
            ErrorKind::SizeLimit => ProgramError::InvalidAccountData,
            _ => ProgramError::GenericError,
        })
    }

    pub fn store(&mut self, new_config_state: ConfigState) -> Result<(), ProgramError> {
        if new_config_state.seq <= self.seq {
            warn!(
                "sequence number old ({} <= {})",
                new_config_state.seq, self.seq
            );
            Err(ProgramError::InvalidArgument)?;
        }

        *self = new_config_state;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_serialize() {
        let mut buffer: Vec<u8> = vec![0; ConfigState::max_size(42) as usize];
        let config_state = ConfigState::new(42);
        config_state.serialize(&mut buffer).unwrap();
        assert_eq!(ConfigState::deserialize(&buffer).unwrap(), config_state);
    }

    #[test]
    fn test_config_store_ok_seq_plus_1() {
        let mut config_state = ConfigState::new(42);
        let mut new_config_state = ConfigState::new(42);
        new_config_state.seq = config_state.seq + 1;
        config_state.store(new_config_state).unwrap();
    }

    #[test]
    fn test_config_store_ok_seq_plus_2() {
        let mut config_state = ConfigState::new(42);
        let mut new_config_state = ConfigState::new(42);
        new_config_state.seq = config_state.seq + 2;
        config_state.store(new_config_state).unwrap();
    }

    #[test]
    fn test_config_store_ok_different_data_len() {
        let mut config_state = ConfigState::new(42);
        let mut new_config_state = ConfigState::new(41);
        new_config_state.seq = config_state.seq + 1;
        config_state.store(new_config_state).unwrap();
    }
    #[test]
    fn test_config_store_fail_old_sequence_number() {
        let mut config_state = ConfigState::new(42);
        let mut new_config_state = ConfigState::new(42);
        new_config_state.seq = config_state.seq;
        config_state.store(new_config_state).unwrap_err();
    }
}
