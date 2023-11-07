// https://github.com/lightningdevkit/rust-lightning/blob/main/lightning-transaction-sync/src/error.rs

use std::fmt;

#[derive(Debug)]
/// An error that possibly needs to be handled by the user.
pub enum TxSyncError {
	/// A transaction sync failed and needs to be retried eventually.
	Failed,
}

impl std::error::Error for TxSyncError {}

impl fmt::Display for TxSyncError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			Self::Failed => write!(f, "Failed to conduct transaction sync."),
		}
	}
}

impl From<bdk::Error> for TxSyncError {
	fn from(_e: bdk::Error) -> Self {
		Self::Failed
	}
}

impl From<crate::error::Error> for TxSyncError {
	fn from(_e: crate::error::Error) -> Self {
		Self::Failed
	}
}

#[cfg(feature = "bitcoind")]
impl From<bitcoincore_rpc::Error> for TxSyncError {
	fn from(_e: bitcoincore_rpc::Error) -> Self {
		Self::Failed
	}
}

#[derive(Debug)]
pub(crate) enum InternalError {
	/// A transaction sync failed and needs to be retried eventually.
	Failed,
	/// An inconsistency was encountered during transaction sync.
	Inconsistency,
}

impl fmt::Display for InternalError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			Self::Failed => write!(f, "Failed to conduct transaction sync."),
			Self::Inconsistency => {
				write!(f, "Encountered an inconsistency during transaction sync.")
			}
		}
	}
}

impl std::error::Error for InternalError {}

#[cfg(feature = "bitcoind")]
impl From<bitcoincore_rpc::Error> for InternalError {
	fn from(_e: bitcoincore_rpc::Error) -> Self {
		Self::Failed
	}
}

impl From<bitcoin::consensus::encode::Error> for InternalError {
	fn from(_e: bitcoin::consensus::encode::Error) -> Self {
		Self::Failed
	}
}

impl From<bitcoin::blockdata::block::Bip34Error> for InternalError {
	fn from(_e: bitcoin::blockdata::block::Bip34Error) -> Self {
		Self::Failed
	}
}

impl From<esplora_client::Error> for TxSyncError {
	fn from(_e: esplora_client::Error) -> Self {
		Self::Failed
	}
}

impl From<esplora_client::Error> for InternalError {
	fn from(_e: esplora_client::Error) -> Self {
		Self::Failed
	}
}

impl From<InternalError> for TxSyncError {
	fn from(_e: InternalError) -> Self {
		Self::Failed
	}
}
