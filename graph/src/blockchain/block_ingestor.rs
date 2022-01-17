use crate::{
    blockchain::{BlockHash, BlockPtr, Blockchain, IngestorAdapter, IngestorError},
    prelude::{info, lazy_static, tokio, trace, warn, Error, LogCode, Logger},
    task_spawn::{block_on, spawn_blocking_allow_panic},
};
use std::{cmp::Ordering, sync::Arc, time::Duration};
use web3::types::H256;

lazy_static! {
    // graph_node::config disallows setting this in a store with multiple
    // shards. See 8b6ad0c64e244023ac20ced7897fe666 for the reason
    pub static ref CLEANUP_BLOCKS: bool = std::env::var("GRAPH_ETHEREUM_CLEANUP_BLOCKS")
        .ok()
        .map(|s| s.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
}

pub struct BlockIngestor<C>
where
    C: Blockchain,
{
    adapter: Arc<C::IngestorAdapter>,
    logger: Logger,
    polling_interval: Duration,
}

impl<C> BlockIngestor<C>
where
    C: Blockchain,
{
    pub fn new(
        adapter: Arc<C::IngestorAdapter>,
        polling_interval: Duration,
    ) -> Result<BlockIngestor<C>, Error> {
        let logger = adapter.logger().clone();
        Ok(BlockIngestor {
            adapter,
            logger,
            polling_interval,
        })
    }

    pub async fn early_into_polling_stream(self) {
        loop {
            match self.early_do_poll().await {
                // Some polls will fail due to transient issues
                Err(err @ IngestorError::BlockUnavailable(_)) => {
                    info!(
                        self.logger,
                        "Trying again after block polling failed: {}", err
                    );
                }
                Err(err @ IngestorError::ReceiptUnavailable(_, _)) => {
                    info!(
                        self.logger,
                        "Trying again after block polling failed: {}", err
                    );
                }
                Err(IngestorError::Unknown(inner_err)) => {
                    warn!(
                        self.logger,
                        "Trying again after block polling failed: {}", inner_err
                    );
                }
                Err(IngestorError::EarlyBlockUninitialized()) => {
                    warn!(self.logger, "Trying again after early block initialized");
                }
                Err(IngestorError::EarlyBlockFinished(_)) => {
                    warn!(self.logger, "Syncing early block finished");
                    break;
                }
                _ => (),
            }

            // if *CLEANUP_BLOCKS {
            //     self.cleanup_cached_blocks()
            // }

            tokio::time::sleep(self.polling_interval).await;
        }
    }

    pub async fn into_polling_stream(self) {
        loop {
            match self.do_poll().await {
                // Some polls will fail due to transient issues
                Err(err @ IngestorError::BlockUnavailable(_)) => {
                    info!(
                        self.logger,
                        "Trying again after block polling failed: {}", err
                    );
                }
                Err(err @ IngestorError::ReceiptUnavailable(_, _)) => {
                    info!(
                        self.logger,
                        "Trying again after block polling failed: {}", err
                    );
                }
                Err(IngestorError::Unknown(inner_err)) => {
                    warn!(
                        self.logger,
                        "Trying again after block polling failed: {}", inner_err
                    );
                }
                Ok(()) => (),
                _ => (),
            }

            if *CLEANUP_BLOCKS {
                self.cleanup_cached_blocks()
            }

            tokio::time::sleep(self.polling_interval).await;
        }
    }

    fn cleanup_cached_blocks(&self) {
        match self.adapter.cleanup_cached_blocks() {
            Ok(Some((min_block, count))) => {
                if count > 0 {
                    info!(
                        self.logger,
                        "Cleaned {} blocks from the block cache. \
                                 Only blocks with number greater than {} remain",
                        count,
                        min_block
                    );
                }
            }
            Ok(None) => { /* nothing was cleaned, ignore */ }
            Err(e) => warn!(
                self.logger,
                "Failed to clean blocks from block cache: {}", e
            ),
        }
    }

    async fn early_do_poll(&self) -> Result<i32, IngestorError> {
        trace!(self.logger, "BlockIngestor::early_do_poll");

        let early_head_block_ptr_opt = self.adapter.chain_early_head_ptr()?;
        let early_head_block_ptr = match early_head_block_ptr_opt {
            None => {
                // Get chain head ptr from store
                let head_block_ptr_opt = self.adapter.chain_head_ptr()?;
                match head_block_ptr_opt {
                    None => {
                        return Err(IngestorError::EarlyBlockUninitialized());
                    }
                    Some(x) => x,
                }
            }
            Some(x) => {
                if x.number == 0 {
                    return Err(IngestorError::EarlyBlockFinished(H256::from_slice(
                        x.hash.as_slice(),
                    )));
                }
                x
            }
        };

        let task_count = self.adapter.early_block_task_count();
        let early_head_block_num = early_head_block_ptr.number;
        let blocks_needed = task_count.min(early_head_block_num);
        let start_head_block_num = early_head_block_num - blocks_needed;

        info!(
            self.logger,
            "Early Syncing [{} to {})...", start_head_block_num, early_head_block_num
        );

        let futures = (start_head_block_num..early_head_block_num)
            .rev()
            .into_iter()
            .map(|block_num| {
                let adapter = Arc::clone(&self.adapter);
                spawn_blocking_allow_panic(move || block_on(adapter.early_ingest_block(block_num)))
            })
            .collect::<Vec<_>>();
        let stored_blocks = futures03::future::join_all(futures)
            .await
            .into_iter()
            .map(|x| {
                x.expect("block_on future:")
                    .expect("early_ingest_block:")
                    .unwrap()
            })
            .collect::<Vec<_>>();

        let (earliest_block_num, earliest_block_hash, _) = stored_blocks
            .into_iter()
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Less))
            .unwrap();
        // todo: check blocks parent hash

        // update synced early head
        self.adapter
            .early_ingest_block_head_update(earliest_block_num.clone(), earliest_block_hash)
            .await?;

        info!(
            self.logger,
            "Early Syncing finished, The next band begin on {} ",
            earliest_block_num.clone()
        );
        Ok(earliest_block_num)
    }

    async fn do_poll(&self) -> Result<(), IngestorError> {
        trace!(self.logger, "BlockIngestor::do_poll");

        // Get chain head ptr from store
        let head_block_ptr_opt = self.adapter.chain_head_ptr()?;

        // To check if there is a new block or not, fetch only the block header since that's cheaper
        // than the full block. This is worthwhile because most of the time there won't be a new
        // block, as we expect the poll interval to be much shorter than the block time.
        let latest_block = self.adapter.latest_block().await?;

        // If latest block matches head block in store, nothing needs to be done
        if Some(&latest_block) == head_block_ptr_opt.as_ref() {
            return Ok(());
        }

        // Compare latest block with head ptr, alert user if far behind
        match head_block_ptr_opt {
            None => {
                info!(
                    self.logger,
                    "Downloading latest blocks from Ethereum. \
                                    This may take a few minutes..."
                );
            }
            Some(head_block_ptr) => {
                let latest_number = latest_block.number;
                let head_number = head_block_ptr.number;
                let distance = latest_number - head_number;
                let blocks_needed = (distance).min(self.adapter.ancestor_count());
                let code = if distance >= 15 {
                    LogCode::BlockIngestionLagging
                } else {
                    LogCode::BlockIngestionStatus
                };
                if distance > 0 {
                    info!(
                        self.logger,
                        "Syncing {} blocks from Ethereum.",
                        blocks_needed;
                        "current_block_head" => head_number,
                        "latest_block_head" => latest_number,
                        "blocks_behind" => distance,
                        "blocks_needed" => blocks_needed,
                        "code" => code,
                    );
                }
            }
        }

        // Store latest block in block store.
        // Might be a no-op if latest block is one that we have seen.
        // ingest_blocks will return a (potentially incomplete) list of blocks that are
        // missing.
        let mut missing_block_hash = self.adapter.ingest_block(&latest_block.hash).await?;

        // Repeatedly fetch missing parent blocks, and ingest them.
        // ingest_blocks will continue to tell us about more missing parent
        // blocks until we have filled in all missing pieces of the
        // blockchain in the block number range we care about.
        //
        // Loop will terminate because:
        // - The number of blocks in the ChainStore in the block number
        //   range [latest - ancestor_count, latest] is finite.
        // - The missing parents in the first iteration have at most block
        //   number latest-1.
        // - Each iteration loads parents of all blocks in the range whose
        //   parent blocks are not already in the ChainStore, so blocks
        //   with missing parents in one iteration will not have missing
        //   parents in the next.
        // - Therefore, if the missing parents in one iteration have at
        //   most block number N, then the missing parents in the next
        //   iteration will have at most block number N-1.
        // - Therefore, the loop will iterate at most ancestor_count times.
        while let Some(hash) = missing_block_hash {
            missing_block_hash = self.adapter.ingest_block(&hash).await?;
        }
        Ok(())
    }

    // for balance
    pub async fn into_polling_balance(self) {
        loop {
            match self.do_poll_balance().await {
                // Some polls will fail due to transient issues
                Err(err @ IngestorError::BlockUnavailable(_)) => {
                    info!(
                        self.logger,
                        "Trying again after block polling failed: {}", err
                    );
                }
                Err(err @ IngestorError::ReceiptUnavailable(_, _)) => {
                    info!(
                        self.logger,
                        "Trying again after block polling failed: {}", err
                    );
                }
                Err(IngestorError::Unknown(inner_err)) => {
                    warn!(
                        self.logger,
                        "Trying again after block polling failed: {}", inner_err
                    );
                }
                Err(IngestorError::EarlyBlockUninitialized()) => {
                    warn!(self.logger, "Trying again after early block initialized");
                }
                Err(IngestorError::EarlyBlockFinished(_)) => {
                    warn!(self.logger, "Syncing early block finished");
                    break;
                }
                _ => continue,
            }

            // todo:
            // if *CLEANUP_BLOCKS {
            //     self.cleanup_cached_blocks()
            // }

            tokio::time::sleep(self.polling_interval).await;
        }
    }

    fn balance_block_ptr(&self) -> Result<(Option<BlockPtr>, Option<BlockPtr>), IngestorError> {
        trace!(self.logger, "BlockIngestor::balance_block_ptr");

        let store = self.adapter.balance_chain_store();

        let head_block_ptr_opt = self.adapter.chain_head_ptr()?;
        let early_head_block_ptr_opt = self.adapter.chain_early_head_ptr()?;
        if head_block_ptr_opt.is_none() || early_head_block_ptr_opt.is_none() {
            return Err(IngestorError::EarlyBlockUninitialized());
        }
        let head_block_ptr = head_block_ptr_opt.unwrap();
        let mut early_head_block_ptr = early_head_block_ptr_opt.unwrap();

        // todo: option
        let balance_head_ptr_opt = store.chain_balance_head_ptr()?;
        let balance_early_head_ptr_opt = store.chain_balance_early_head_ptr()?;

        let balance_head_ptr = balance_head_ptr_opt.unwrap_or(BlockPtr {
            number: i32::MAX,
            hash: head_block_ptr.clone().hash,
        });
        let balance_early_head_ptr = balance_early_head_ptr_opt.unwrap_or(BlockPtr {
            number: 0,
            hash: head_block_ptr.clone().hash,
        });

        if head_block_ptr.number > balance_head_ptr.number
            && early_head_block_ptr.number < balance_early_head_ptr.number
        {
            return Ok((Some(balance_head_ptr), Some(balance_early_head_ptr)));
        } else if head_block_ptr.number > balance_head_ptr.number {
            return Ok((Some(balance_head_ptr), None));
        } else if early_head_block_ptr.number < balance_early_head_ptr.number {
            return Ok((None, Some(balance_early_head_ptr)));
        }
        early_head_block_ptr.number = head_block_ptr.number - 1;
        early_head_block_ptr.hash = head_block_ptr.hash.clone();
        return Ok((Some(head_block_ptr), Some(early_head_block_ptr)));
    }

    async fn do_poll_balance(&self) -> Result<Option<i32>, IngestorError> {
        trace!(self.logger, "BlockIngestor::do_poll_balance");

        let store = self.adapter.balance_chain_store();
        let adapter = Arc::clone(&self.adapter);
        let (balance_block_ptr_opt, balance_early_block_ptr_opt) = self.balance_block_ptr()?;

        // todo: revert
        // backward
        if balance_early_block_ptr_opt.is_some() {
            let balance_early_block_ptr = balance_early_block_ptr_opt.clone().unwrap();
            let backward_adapter = Arc::clone(&self.adapter);
            let backward_task = spawn_blocking_allow_panic(move || -> Result<i32, Error> {
                let rst = block_on(backward_adapter.balance_ingest(balance_early_block_ptr))?;
                Ok(rst)
            });
            let rst = backward_task.await;
            match rst {
                Err(e) => return Err(IngestorError::Unknown(Error::from(e))),
                Ok(cnt) => {
                    // let cnt = cnt?;

                    let number = balance_early_block_ptr_opt.as_ref().unwrap().block_number() - 1;
                    let next_ptr = adapter
                        .blockptr_by_number(number)
                        .await
                        .expect(format!("balance no block:{}", number).as_str());
                    store
                        .chain_update_balance_early_head(&next_ptr)
                        .await
                        .map_err(|e| IngestorError::Unknown(e))?;
                }
            }
        }
        // forward
        if balance_block_ptr_opt.is_some() {
            let balance_block_ptr = balance_block_ptr_opt.clone().unwrap();
            let forward_adapter = Arc::clone(&self.adapter);
            let forward_task = spawn_blocking_allow_panic(move || -> Result<i32, Error> {
                let rst = block_on(forward_adapter.balance_ingest(balance_block_ptr))?;
                Ok(rst)
            });
            let rst = forward_task.await;
            match rst {
                Err(e) => return Err(IngestorError::Unknown(Error::from(e))),
                Ok(cnt) => {
                    // let cnt = cnt?;

                    let number = balance_block_ptr_opt.as_ref().unwrap().block_number() + 1;
                    let next_ptr = adapter
                        .blockptr_by_number(number)
                        .await
                        .expect(format!("balance no block:{}", number).as_str());
                    store
                        .chain_update_balance_head(&next_ptr)
                        .await
                        .map_err(|e| IngestorError::Unknown(e))?;
                }
            }
        }

        return Ok(Some(1));
    }
}
