mod common;
pub mod error;

#[cfg(feature = "esplora-async")]
pub mod esplora;

#[cfg(feature = "bitcoind")]
pub mod bitcoind;

pub mod blockchain;

use bitcoin::{Script, Txid};
use common::{FilterQueue, SyncState};
use core::ops::Deref;
use lightning::chain::Filter;
use lightning::chain::WatchedOutput;
use lightning::util::logger::Logger;

#[cfg(any(feature = "esplora-async", feature = "bitcoind"))]
pub type MutexType<I> = futures::lock::Mutex<I>;
// #[cfg(feature = "bitcoind")]
// pub type MutexType<I> = std::sync::Mutex<I>;

// The underlying client type.
#[cfg(feature = "esplora-async")]
pub type ClientType = esplora_client::AsyncClient;
#[cfg(feature = "bitcoind")]
pub type ClientType = bdk::bitcoincore_rpc::Client;

/// Synchronizes LDK with a given [`Esplora`] server.
///
/// Needs to be registered with a [`ChainMonitor`] via the [`Filter`] interface to be informed of
/// transactions and outputs to monitor for on-chain confirmation, unconfirmation, and
/// reconfirmation.
///
/// Note that registration via [`Filter`] needs to happen before any calls to
/// [`Watch::watch_channel`] to ensure we get notified of the items to monitor.
///
/// This uses and exposes either a blocking or async client variant dependent on whether the
/// `esplora-blocking` or the `esplora-async` feature is enabled.
///
/// [`Esplora`]: https://github.com/Blockstream/electrs
/// [`ChainMonitor`]: lightning::chain::chainmonitor::ChainMonitor
/// [`Watch::watch_channel`]: lightning::chain::Watch::watch_channel
/// [`Filter`]: lightning::chain::Filter
pub struct SyncClient<L: Deref>
where
	L::Target: Logger,
{
	sync_state: MutexType<SyncState>,
	queue: std::sync::Mutex<FilterQueue>,
	client: ClientType,
	logger: L,
}

impl<L: Deref> Filter for SyncClient<L>
where
	L::Target: Logger,
{
	fn register_tx(&self, txid: &Txid, _script_pubkey: &Script) {
		let mut locked_queue = self.queue.lock().unwrap();
		locked_queue.transactions.insert(*txid);
	}

	fn register_output(&self, output: WatchedOutput) {
		let mut locked_queue = self.queue.lock().unwrap();
		locked_queue.outputs.insert(output.outpoint.into_bitcoin_outpoint(), output);
	}
}
