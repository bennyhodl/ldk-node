use super::common::{ConfirmedTx, FilterQueue, SyncState};
use super::error::{InternalError, TxSyncError};
use super::{ClientType, MutexType, SyncClient};

use bitcoin::{BlockHash, Txid};
use esplora_client::Builder;
use lightning::chain::Confirm;
use lightning::util::logger::Logger;
use lightning::{log_debug, log_error, log_info, log_trace};
use std::collections::HashSet;
use std::ops::Deref;

impl<L: Deref> SyncClient<L>
where
	L::Target: Logger,
{
	/// Returns a new [`SyncClient`] object.
	pub fn new(server_url: String, logger: L) -> Self {
		let builder = Builder::new(&server_url);
		let client = builder.build_async().unwrap();
		SyncClient::from_client(client, logger)
	}

	/// Returns a new [`SyncClient`] object using the given Esplora client.
	pub fn from_client(client: ClientType, logger: L) -> Self {
		let sync_state = MutexType::new(SyncState::new());
		let queue = std::sync::Mutex::new(FilterQueue::new());
		Self { sync_state, queue, client, logger }
	}

	/// Synchronizes the given `confirmables` via their [`Confirm`] interface implementations. This
	/// method should be called regularly to keep LDK up-to-date with current chain data.
	///
	/// For example, instances of [`ChannelManager`] and [`ChainMonitor`] can be informed about the
	/// newest on-chain activity related to the items previously registered via the [`Filter`]
	/// interface.
	///
	/// [`Confirm`]: lightning::chain::Confirm
	/// [`ChainMonitor`]: lightning::chain::chainmonitor::ChainMonitor
	/// [`ChannelManager`]: lightning::ln::channelmanager::ChannelManager
	/// [`Filter`]: lightning::chain::Filter
	#[maybe_async]
	#[cfg(feature = "esplora-async")]
	pub async fn sync(
		&self, confirmables: Vec<&(dyn Confirm + Sync + Send)>,
	) -> Result<(), TxSyncError> {
		// This lock makes sure we're syncing once at a time.
		let mut sync_state = self.sync_state.lock().await;

		log_info!(self.logger, "Starting transaction sync.");

		let mut tip_hash = self.client.get_tip_hash().await?;

		loop {
			let pending_registrations = self.queue.lock().unwrap().process_queues(&mut sync_state);

			let tip_is_new = Some(tip_hash) != sync_state.last_sync_hash;

			// We loop until any registered transactions have been processed at least once, or the
			// tip hasn't been updated during the last iteration.
			if !sync_state.pending_sync && !pending_registrations && !tip_is_new {
				// Nothing to do.
				break;
			} else {
				// Update the known tip to the newest one.
				if tip_is_new {
					// First check for any unconfirmed transactions and act on it immediately.
					match self.get_unconfirmed_transactions(&confirmables).await {
						Ok(unconfirmed_txs) => {
							// Double-check the tip hash. If it changed, a reorg happened since
							// we started syncing and we need to restart last-minute.
							let check_tip_hash = self.client.get_tip_hash().await?;

							if check_tip_hash != tip_hash {
								tip_hash = check_tip_hash;
								continue;
							}

							self.sync_unconfirmed_transactions(
								&mut sync_state,
								&confirmables,
								unconfirmed_txs,
							);
						}
						Err(err) => {
							// (Semi-)permanent failure, retry later.
							log_error!(self.logger, "Failed during transaction sync, aborting.");
							sync_state.pending_sync = true;
							return Err(TxSyncError::from(err));
						}
					}

					match self.sync_best_block_updated(&confirmables, &tip_hash).await {
						Ok(()) => {}
						Err(InternalError::Inconsistency) => {
							// Immediately restart syncing when we encounter any inconsistencies.
							log_debug!(
								self.logger,
								"Encountered inconsistency during transaction sync, restarting."
							);
							sync_state.pending_sync = true;
							continue;
						}
						Err(err) => {
							// (Semi-)permanent failure, retry later.
							sync_state.pending_sync = true;
							return Err(TxSyncError::from(err));
						}
					}
				}

				match self.get_confirmed_transactions(&sync_state).await {
					Ok(confirmed_txs) => {
						// Double-check the tip hash. If it changed, a reorg happened since
						// we started syncing and we need to restart last-minute.
						let check_tip_hash = self.client.get_tip_hash().await?;

						if check_tip_hash != tip_hash {
							tip_hash = check_tip_hash;
							continue;
						}

						self.sync_confirmed_transactions(
							&mut sync_state,
							&confirmables,
							confirmed_txs,
						);
					}
					Err(InternalError::Inconsistency) => {
						// Immediately restart syncing when we encounter any inconsistencies.
						log_debug!(
							self.logger,
							"Encountered inconsistency during transaction sync, restarting."
						);
						sync_state.pending_sync = true;
						continue;
					}
					Err(err) => {
						// (Semi-)permanent failure, retry later.
						log_error!(self.logger, "Failed during transaction sync, aborting.");
						sync_state.pending_sync = true;
						return Err(TxSyncError::from(err));
					}
				}
				sync_state.last_sync_hash = Some(tip_hash);
				sync_state.pending_sync = false;
			}
		}
		log_info!(self.logger, "Finished transaction sync.");
		Ok(())
	}

	#[maybe_async]
	async fn sync_best_block_updated(
		&self, confirmables: &Vec<&(dyn Confirm + Sync + Send)>, tip_hash: &BlockHash,
	) -> Result<(), InternalError> {
		// Inform the interface of the new block.
		let tip_header = self.client.get_header_by_hash(tip_hash).await?;

		let tip_status = self.client.get_block_status(&tip_hash).await?;

		if tip_status.in_best_chain {
			if let Some(tip_height) = tip_status.height {
				for c in confirmables {
					c.best_block_updated(&tip_header, tip_height);
				}
			}
		} else {
			return Err(InternalError::Inconsistency);
		}

		Ok(())
	}

	fn sync_confirmed_transactions(
		&self, sync_state: &mut SyncState, confirmables: &Vec<&(dyn Confirm + Sync + Send)>,
		confirmed_txs: Vec<ConfirmedTx>,
	) {
		for ctx in confirmed_txs {
			for c in confirmables {
				c.transactions_confirmed(
					&ctx.block_header,
					&[(ctx.pos, &ctx.tx)],
					ctx.block_height,
				);
			}

			sync_state.watched_transactions.remove(&ctx.tx.txid());

			for input in &ctx.tx.input {
				sync_state.watched_outputs.remove(&input.previous_output);
			}
		}
	}

	#[maybe_async]
	async fn get_confirmed_transactions(
		&self, sync_state: &SyncState,
	) -> Result<Vec<ConfirmedTx>, InternalError> {
		// First, check the confirmation status of registered transactions as well as the
		// status of dependent transactions of registered outputs.

		let mut confirmed_txs = Vec::new();

		for txid in &sync_state.watched_transactions {
			if let Some(confirmed_tx) = self.get_confirmed_tx(&txid, None, None).await? {
				confirmed_txs.push(confirmed_tx);
			}
		}

		for (_, output) in &sync_state.watched_outputs {
			if let Some(output_status) = self
				.client
				.get_output_status(&output.outpoint.txid, output.outpoint.index as u64)
				.await?
			{
				if let Some(spending_txid) = output_status.txid {
					if let Some(spending_tx_status) = output_status.status {
						if let Some(confirmed_tx) = self
							.get_confirmed_tx(
								&spending_txid,
								spending_tx_status.block_hash,
								spending_tx_status.block_height,
							)
							.await?
						{
							confirmed_txs.push(confirmed_tx);
						}
					}
				}
			}
		}

		// Sort all confirmed transactions first by block height, then by in-block
		// position, and finally feed them to the interface in order.
		confirmed_txs.sort_unstable_by(|tx1: &ConfirmedTx, tx2: &ConfirmedTx| {
			tx1.block_height.cmp(&tx2.block_height).then_with(|| tx1.pos.cmp(&tx2.pos))
		});

		Ok(confirmed_txs)
	}

	#[maybe_async]
	async fn get_confirmed_tx(
		&self, txid: &Txid, expected_block_hash: Option<BlockHash>, known_block_height: Option<u32>,
	) -> Result<Option<ConfirmedTx>, InternalError> {
		if let Some(merkle_block) = self.client.get_merkle_block(&txid).await? {
			let block_header = merkle_block.header;
			let block_hash = block_header.block_hash();
			if let Some(expected_block_hash) = expected_block_hash {
				if expected_block_hash != block_hash {
					log_trace!(
						self.logger,
						"Inconsistency: Tx {} expected in block {}, but is confirmed in {}",
						txid,
						expected_block_hash,
						block_hash
					);
					return Err(InternalError::Inconsistency);
				}
			}

			let mut matches = Vec::new();
			let mut indexes = Vec::new();
			let _ = merkle_block.txn.extract_matches(&mut matches, &mut indexes);
			if indexes.len() != 1 || matches.len() != 1 || matches[0] != *txid {
				log_error!(self.logger, "Retrieved Merkle block for txid {} doesn't match expectations. This should not happen. Please verify server integrity.", txid);
				return Err(InternalError::Failed);
			}

			let pos = *indexes.get(0).ok_or(InternalError::Failed)? as usize;
			if let Some(tx) = self.client.get_tx(&txid).await? {
				if let Some(block_height) = known_block_height {
					// We can take a shortcut here if a previous call already gave us the height.
					return Ok(Some(ConfirmedTx { tx, block_header, pos, block_height }));
				}

				let block_status = self.client.get_block_status(&block_hash).await?;
				if let Some(block_height) = block_status.height {
					return Ok(Some(ConfirmedTx { tx, block_header, pos, block_height }));
				} else {
					// If any previously-confirmed block suddenly is no longer confirmed, we found
					// an inconsistency and should start over.
					log_trace!(
						self.logger,
						"Inconsistency: Tx {} was unconfirmed during syncing.",
						txid
					);
					return Err(InternalError::Inconsistency);
				}
			}
		}
		Ok(None)
	}

	#[maybe_async]
	async fn get_unconfirmed_transactions(
		&self, confirmables: &Vec<&(dyn Confirm + Sync + Send)>,
	) -> Result<Vec<Txid>, InternalError> {
		// Query the interface for relevant txids and check whether the relevant blocks are still
		// in the best chain, mark them unconfirmed otherwise
		let relevant_txids = confirmables
			.iter()
			.flat_map(|c| c.get_relevant_txids())
			.collect::<HashSet<(Txid, Option<BlockHash>)>>();

		let mut unconfirmed_txs = Vec::new();

		for (txid, block_hash_opt) in relevant_txids {
			if let Some(block_hash) = block_hash_opt {
				let block_status = self.client.get_block_status(&block_hash).await?;

				if block_status.in_best_chain {
					// Skip if the block in question is still confirmed.
					continue;
				}

				unconfirmed_txs.push(txid);
			} else {
				log_error!(self.logger, "Untracked confirmation of funding transaction. Please ensure none of your channels had been created with LDK prior to version 0.0.113!");
				panic!("Untracked confirmation of funding transaction. Please ensure none of your channels had been created with LDK prior to version 0.0.113!");
			}
		}
		Ok(unconfirmed_txs)
	}

	fn sync_unconfirmed_transactions(
		&self, sync_state: &mut SyncState, confirmables: &Vec<&(dyn Confirm + Sync + Send)>,
		unconfirmed_txs: Vec<Txid>,
	) {
		for txid in unconfirmed_txs {
			for c in confirmables {
				c.transaction_unconfirmed(&txid);
			}

			sync_state.watched_transactions.insert(txid);
		}
	}

	/// Returns a reference to the underlying esplora client.
	pub fn client(&self) -> &ClientType {
		&self.client
	}
}
