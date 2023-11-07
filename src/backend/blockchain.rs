#[cfg(feature = "bitcoind")]
use bdk::blockchain::rpc::RpcBlockchain;
#[cfg(feature = "esplora-async")]
use bdk::blockchain::EsploraBlockchain;

#[cfg(feature = "bitcoind")]
pub type BlockchainType = RpcBlockchain;
#[cfg(feature = "esplora-async")]
pub type BlockchainType = EsploraBlockchain;
