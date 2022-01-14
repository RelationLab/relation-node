use crate::lazy_static::__Deref;
use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, PooledConnection};
use diesel::sql_types::Text;
use diesel::{insert_into, update};
use graph::ensure;
use graph::prelude::web3::types::{Address, H256, U256};
use graph::prelude::BigDecimal;
use graph::{
    constraint_violation,
    prelude::{
        async_trait, ethabi, CancelableError, ChainStore as ChainStoreTrait, EthereumCallCache,
        StoreError,
    },
};
use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    iter::FromIterator,
    sync::Arc,
};

use graph::prelude::{
    transaction_receipt::LightTransactionReceipt, BlockNumber, BlockPtr, Error, EthereumBlock,
    EthereumNetworkIdentifier, LightEthereumBlock,
};

use crate::{
    block_store::ChainStatus, chain_head_listener::ChainHeadUpdateSender,
    connection_pool::ConnectionPool,
};

/// Tables in the 'public' database schema that store chain-specific data
mod public {
    table! {
        ethereum_networks (name) {
            name -> Varchar,
            namespace -> Varchar,
            head_block_hash -> Nullable<Varchar>,
            head_block_number -> Nullable<BigInt>,
            early_head_block_hash -> Nullable<Varchar>,
            early_head_block_number -> Nullable<BigInt>,
            head_updated -> Timestamp,
            early_head_updated -> Timestamp,
            net_version -> Varchar,
            genesis_block_hash -> Varchar,
        }
    }

    table! {
        ethereum_balance (name) {
            name -> Varchar,
            namespace -> Varchar,
            head_block_hash -> Nullable<Varchar>,
            head_block_number -> Nullable<BigInt>,
            early_head_block_hash -> Nullable<Varchar>,
            early_head_block_number -> Nullable<BigInt>,
            head_updated -> Timestamp,
            early_head_updated -> Timestamp,
            net_version -> Varchar,
            genesis_block_hash -> Varchar,
        }
    }
}

pub use data::Storage;

/// Encapuslate access to the blocks table for a chain.
mod data {

    use diesel::sql_types::Binary;
    use diesel::{connection::SimpleConnection, insert_into};
    use diesel::{delete, prelude::*, sql_query};
    use diesel::{dsl::sql, pg::PgConnection};
    use diesel::{
        pg::Pg,
        serialize::Output,
        sql_types::Text,
        types::{FromSql, ToSql},
    };
    use diesel::{
        sql_types::{BigInt, Bytea, Integer, Jsonb, Nullable, Numeric},
        update,
    };
    use diesel_dynamic_schema as dds;
    use graph::{
        constraint_violation,
        prelude::{transaction_receipt::LightTransactionReceipt, StoreError},
    };

    use core::any::type_name;
    use graph::prelude::BigDecimal;
    use graph::prelude::{
        serde_json, web3::types::Bytes, web3::types::H160, web3::types::H256, web3::types::U256,
        web3::types::U64, BlockNumber, BlockPtr, Error, EthereumBlock, LightEthereumBlock,
    };
    use std::any::Any;
    use std::fmt;
    use std::iter::FromIterator;
    use std::sync::Arc;
    use std::{convert::TryFrom, io::Write};
    // use std::any::TypeId;

    struct BindSqlType<T> {
        _val: T,
    }
    impl<T: Any> BindSqlType<T> {
        pub fn bytea(input: T) -> String {
            let any = &input as &dyn Any;

            if let Some(x) = any.downcast_ref::<H256>() {
                return format!("decode('{:x}', 'hex')", x);
            } else if let Some(x) = any.downcast_ref::<H160>() {
                return format!("decode('{:x}', 'hex')", x);
            } else if let Some(x) = any.downcast_ref::<Bytes>() {
                let mut s = hex::encode(&x.0);
                if s.len() % 2 == 1 {
                    s = format!("0{}", s);
                }
                return format!("decode('{}', 'hex')", s);
            } else if let Some(x) = any.downcast_ref::<U64>() {
                let mut s = format!("{:x}", x);
                if s.len() % 2 == 1 {
                    s = format!("0{}", s);
                }
                return format!("decode('{}', 'hex')", s);
            } else if let Some(x) = any.downcast_ref::<U256>() {
                let mut s = format!("{:x}", x);
                if s.len() % 2 == 1 {
                    s = format!("0{}", s);
                }
                return format!("decode('{}', 'hex')", s);
            } else {
                panic!("unsuppot bindBytea type: {}", type_name::<T>());
            }
        }
    }

    use crate::transaction_receipt::RawTransactionReceipt;

    pub(crate) const ETHEREUM_BLOCKS_TABLE_NAME: &'static str = "public.ethereum_blocks";

    mod public {
        pub(super) use super::super::public::ethereum_balance;
        pub(super) use super::super::public::ethereum_networks;

        //chain table
        table! {
            chain_transactions (hash) {
                block_hash -> Varchar,
                block_number -> BigInt,
                from -> Varchar,
                gas -> Varchar,
                gas_price -> Varchar,
                max_fee_per_gas -> Nullable<BigInt>,
                max_priority_fe_per_gas -> Nullable<BigInt>,
                hash -> Varchar,
                input -> Text,
                nonce -> Varchar,
                transaction_index -> Varchar,
                value -> Varchar,
            }
        }
        table! {
            chain_receipts (id) {
                /// `id` is the transaction_hash + log_index
                id -> Bytea,
                // data -> Varchar,
                // topics -> Text,
                // address -> Varchar,
                // log_type -> Nullable<Varchar>,
                // removed -> Nullable<Bool>,
                // log_index -> Nullable<Varchar>,
                // block_hash -> Bytea,
                // block_number -> Nullable<BigInt>,
                // transaction_hash -> Nullable<Varchar>,
                // transaction_index -> Nullable<Varchar>,
                // transaction_log_index -> Nullable<Varchar>,
            }
        }

        table! {
            ethereum_blocks (hash) {
                hash -> Varchar,
                number -> BigInt,
                parent_hash -> Nullable<Varchar>,
                network_name -> Varchar, // REFERENCES ethereum_networks (name),
                data -> Jsonb,
            }
        }
        allow_tables_to_appear_in_same_query!(ethereum_networks, ethereum_blocks);

        table! {
            ethereum_transactions (hash) {
                block_hash -> Varchar,
                block_number -> BigInt,
                from -> Varchar,
                gas -> Varchar,
                gas_price -> Varchar,
                max_fee_per_gas -> Nullable<BigInt>,
                max_priority_fee_per_gas -> Nullable<BigInt>,
                hash -> Varchar,
                input -> Text,
                nonce -> Varchar,
                transaction_index -> Varchar,
                value -> Varchar,
            }
        }
        allow_tables_to_appear_in_same_query!(ethereum_networks, ethereum_transactions);

        table! {
            ethereum_receipts (id) {
                /// `id` is the transaction_hash + log_index
                id -> Bytea,
                data -> Varchar,
                topics -> Text,
                address -> Varchar,
                log_type -> Nullable<Varchar>,
                removed -> Bool,
                log_index -> BigInt,
                block_hash -> Varchar,
                block_number -> BigInt,
                transaction_hash -> Varchar,
                transaction_index -> Varchar,
                transaction_log_index -> Nullable<Varchar>,
            }
        }
        allow_tables_to_appear_in_same_query!(ethereum_networks, ethereum_receipts);

        table! {
            /// `id` is the hash of contract address + encoded function call + block number.
            eth_call_cache (id) {
                id -> Bytea,
                return_value -> Bytea,
                contract_address -> Bytea,
                block_number -> Integer,
            }
        }

        table! {
            /// When was a cached call on a contract last used? This is useful to clean old data.
            eth_call_meta (contract_address) {
                contract_address -> Bytea,
                accessed_at -> Date,
            }
        }

        joinable!(eth_call_cache -> eth_call_meta (contract_address));
        allow_tables_to_appear_in_same_query!(eth_call_cache, eth_call_meta);
    }

    // Helper for literal SQL queries that look up a block hash
    #[derive(QueryableByName)]
    struct BlockHashText {
        #[sql_type = "Text"]
        hash: String,
    }

    #[derive(QueryableByName)]
    struct BlockHashBytea {
        #[sql_type = "Bytea"]
        hash: Vec<u8>,
    }

    // Like H256::from_slice, but returns an error instead of panicking
    // when `bytes` does not have the right length
    fn h256_from_bytes(bytes: &[u8]) -> Result<H256, StoreError> {
        if bytes.len() == H256::len_bytes() {
            Ok(H256::from_slice(bytes))
        } else {
            Err(constraint_violation!(
                "invalid H256 value `{}` has {} bytes instead of {}",
                graph::prelude::hex::encode(bytes),
                bytes.len(),
                H256::len_bytes()
            ))
        }
    }

    type DynTable = dds::Table<String>;
    type DynColumn<ST> = dds::Column<DynTable, &'static str, ST>;

    /// The table that holds blocks when we store a chain in its own
    /// dedicated database schema
    #[derive(Clone, Debug)]
    struct BlocksTable {
        /// The fully qualified name of the blocks table, including the
        /// schema
        qname: String,
        table: DynTable,
    }

    impl BlocksTable {
        const TABLE_NAME: &'static str = "blocks";

        fn new(namespace: &str) -> Self {
            BlocksTable {
                qname: format!("{}.{}", namespace, Self::TABLE_NAME),
                table: dds::schema(namespace.to_string()).table(Self::TABLE_NAME.to_string()),
            }
        }

        fn table(&self) -> DynTable {
            self.table.clone()
        }

        fn hash(&self) -> DynColumn<Bytea> {
            self.table.column::<Bytea, _>("hash")
        }

        fn number(&self) -> DynColumn<BigInt> {
            self.table.column::<BigInt, _>("number")
        }

        fn data(&self) -> DynColumn<Jsonb> {
            self.table.column::<Jsonb, _>("data")
        }
    }

    #[derive(Clone, Debug)]
    struct TransactionsTable {
        /// The fully qualified name of the blocks table, including the
        /// schema
        qname: String,
        table: DynTable,
    }

    impl TransactionsTable {
        const TABLE_NAME: &'static str = "transactions";

        fn new(namespace: &str) -> Self {
            TransactionsTable {
                qname: format!("{}.{}", namespace, Self::TABLE_NAME),
                table: dds::schema(namespace.to_string()).table(Self::TABLE_NAME.to_string()),
            }
        }

        fn to(&self) -> DynColumn<Bytea> {
            self.table.column::<Bytea, _>("to")
        }
        fn table(&self) -> DynTable {
            self.table.clone()
        }
        fn block_number(&self) -> DynColumn<BigInt> {
            self.table.column::<BigInt, _>("block_number")
        }
    }
    #[derive(Clone, Debug)]
    struct BalanceTable {
        /// The fully qualified name of the blocks table, including the
        /// schema
        qname: String,
        table: DynTable,
    }
    impl BalanceTable {
        const TABLE_NAME: &'static str = "balance";

        fn new(namespace: &str) -> Self {
            BalanceTable {
                qname: format!("{}.{}", namespace, Self::TABLE_NAME),
                table: dds::schema(namespace.to_string()).table(Self::TABLE_NAME.to_string()),
            }
        }

        fn table(&self) -> DynTable {
            self.table.clone()
        }
    }
    #[derive(Clone, Debug)]
    struct ReceiptsTable {
        /// The fully qualified name of the blocks table, including the
        /// schema
        qname: String,
        table: DynTable,
    }

    impl ReceiptsTable {
        const TABLE_NAME: &'static str = "receipts";

        fn new(namespace: &str) -> Self {
            ReceiptsTable {
                qname: format!("{}.{}", namespace, Self::TABLE_NAME),
                table: dds::schema(namespace.to_string()).table(Self::TABLE_NAME.to_string()),
            }
        }

        // fn table(&self) -> DynTable {
        //     self.table.clone()
        // }
    }

    #[derive(Clone, Debug)]
    struct CallMetaTable {
        qname: String,
        table: DynTable,
    }

    impl CallMetaTable {
        const TABLE_NAME: &'static str = "call_meta";
        const ACCESSED_AT: &'static str = "accessed_at";

        fn new(namespace: &str) -> Self {
            CallMetaTable {
                qname: format!("{}.{}", namespace, Self::TABLE_NAME),
                table: dds::schema(namespace.to_string()).table(Self::TABLE_NAME.to_string()),
            }
        }

        fn table(&self) -> DynTable {
            self.table.clone()
        }

        fn contract_address(&self) -> DynColumn<Bytea> {
            self.table.column::<Bytea, _>("contract_address")
        }
    }

    #[derive(Clone, Debug)]
    struct CallCacheTable {
        qname: String,
        table: DynTable,
    }

    impl CallCacheTable {
        const TABLE_NAME: &'static str = "call_cache";

        fn new(namespace: &str) -> Self {
            CallCacheTable {
                qname: format!("{}.{}", namespace, Self::TABLE_NAME),
                table: dds::schema(namespace.to_string()).table(Self::TABLE_NAME.to_string()),
            }
        }

        fn table(&self) -> DynTable {
            self.table.clone()
        }

        fn id(&self) -> DynColumn<Bytea> {
            self.table.column::<Bytea, _>("id")
        }

        fn return_value(&self) -> DynColumn<Bytea> {
            self.table.column::<Bytea, _>("return_value")
        }

        fn contract_address(&self) -> DynColumn<Bytea> {
            self.table.column::<Bytea, _>("contract_address")
        }
    }

    #[derive(Clone, Debug)]
    pub struct Schema {
        name: String,
        blocks: BlocksTable,
        transactions: TransactionsTable,
        receipts: ReceiptsTable,
        call_meta: CallMetaTable,
        call_cache: CallCacheTable,
        balance: BalanceTable,
    }

    impl Schema {
        fn new(name: String) -> Self {
            let blocks = BlocksTable::new(&name);
            let transactions = TransactionsTable::new(&name);
            let receipts = ReceiptsTable::new(&name);
            let call_meta = CallMetaTable::new(&name);
            let call_cache = CallCacheTable::new(&name);
            let balance = BalanceTable::new(&name);

            Self {
                name,
                blocks,
                transactions,
                receipts,
                call_meta,
                call_cache,
                balance,
            }
        }
    }

    #[derive(Clone, Debug, AsExpression, FromSqlRow)]
    #[sql_type = "diesel::sql_types::Text"]
    /// Storage for a chain. The underlying namespace (database schema) is either
    /// `public` or of the form `chain[0-9]+`.
    pub enum Storage {
        /// Chain data is stored in shared tables
        Shared,
        /// The chain has its own namespace in the database with dedicated
        /// tables
        Private(Schema),
    }

    impl fmt::Display for Storage {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            match self {
                Self::Shared => Self::PUBLIC.fmt(f),
                Self::Private(Schema { name, .. }) => name.fmt(f),
            }
        }
    }

    impl FromSql<Text, Pg> for Storage {
        fn from_sql(bytes: Option<&[u8]>) -> diesel::deserialize::Result<Self> {
            let s = <String as FromSql<Text, Pg>>::from_sql(bytes)?;
            Self::new(s).map_err(Into::into)
        }
    }

    impl ToSql<Text, Pg> for Storage {
        fn to_sql<W: Write>(&self, out: &mut Output<W, Pg>) -> diesel::serialize::Result {
            <String as ToSql<Text, Pg>>::to_sql(&self.to_string(), out)
        }
    }

    impl Storage {
        const PREFIX: &'static str = "chain";
        const PUBLIC: &'static str = "public";

        fn new(s: String) -> Result<Self, String> {
            if s.as_str() == Self::PUBLIC {
                return Ok(Self::Shared);
            }

            if !s.starts_with(Self::PREFIX) || s.len() <= Self::PREFIX.len() {
                return Err(s);
            }
            for c in s.chars().skip(Self::PREFIX.len()) {
                if !c.is_numeric() {
                    return Err(s);
                }
            }

            Ok(Self::Private(Schema::new(s)))
        }

        /// Create dedicated database tables for this chain if it uses
        /// `Storage::Private`. If it uses `Storage::Shared`, do nothing since
        /// a regular migration will already have created the `ethereum_blocks`
        /// table
        pub(super) fn create(&self, conn: &PgConnection) -> Result<(), Error> {
            fn make_ddl(nsp: &str) -> String {
                format!(
                    "
                create schema {nsp};
                create table {nsp}.blocks (
                  hash         bytea  not null primary key,
                  number       int8  not null,
                  parent_hash  bytea  not null,
                  data         jsonb not null
                );
                create index blocks_number ON {nsp}.blocks using btree(number);

                create table {nsp}.transactions (
                  hash                      bytea not null primary key,
                  transaction_index         bytea not null,
                  block_hash                bytea not null,
                  block_number              int8 not null,
                  gas                       int8 not null,
                  gas_price                 int8 not null,
                  max_fee_per_gas           int8,
                  max_priority_fe_per_gas   int8,
                  input                     bytea not null,
                  \"from\"                  bytea not null,
                  \"to\"                    bytea,
                  trx_type                  int8,
                  nonce                     bytea not null,
                  value                     bytea not null
                );
                create index tx_hash ON {nsp}.transactions using btree(hash);

                create table {nsp}.receipts (
                  id                    bytea not null primary key,
                  block_hash            bytea,
                  block_number          int8,
                  data                  bytea  not null,
                  topics                text[],
                  address               bytea,
                  removed               bool,
                  log_index             bytea,
                  log_type              int8,
                  transaction_hash      bytea,
                  transaction_index     bytea not null,
                  cumulative_gas_used   int8,
                  effective_gas_used    int8,
                  gas_used              int8,
                  \"from\"              bytea,
                  \"to\"                bytea
                );

                create index tx_receipt_index ON {nsp}.receipts using btree(transaction_hash, log_index);

                create table {nsp}.call_cache (
	              id               bytea not null primary key,
	              return_value     bytea not null,
	              contract_address bytea not null,
	              block_number     int4 not null,
                  method_id         bytea not null,
                  method_params     Text
                );
                comment on column {nsp}.call_cache.method_params is 'call-params split by ,';


                create table {nsp}.balance (
                    block_hash            bytea,
                    block_number          int8,
                    amount                numeric,
                    address               bytea,
                    primary key(address, block_number)
                );
                create index address_number_index ON {nsp}.balance using btree(address, block_number);


                create table {nsp}.call_meta (
                    contract_address bytea not null primary key,
                    accessed_at      date  not null
                );
            ",
                    nsp = nsp
                )
            }

            match self {
                Storage::Shared => Ok(()),
                Storage::Private(Schema { name, .. }) => {
                    conn.batch_execute(&make_ddl(name))?;
                    Ok(())
                }
            }
        }

        /// Returns a fully qualified table name to the blocks table
        #[inline]
        fn blocks_table(&self) -> &str {
            match self {
                Storage::Shared => ETHEREUM_BLOCKS_TABLE_NAME,
                Storage::Private(Schema { blocks, .. }) => &blocks.qname,
            }
        }

        pub(super) fn drop_storage(
            &self,
            conn: &PgConnection,
            name: &str,
        ) -> Result<(), StoreError> {
            match &self {
                Storage::Shared => {
                    use public::ethereum_blocks as b;
                    delete(b::table.filter(b::network_name.eq(name))).execute(conn)?;
                    Ok(())
                }
                Storage::Private(Schema { name, .. }) => {
                    conn.batch_execute(&format!("drop schema {} cascade", name))?;
                    Ok(())
                }
            }
        }

        pub(super) fn upsert_balance(
            &self,
            conn: &PgConnection,
            address: &web3::types::Address,
            amount: U256,
            block_ptr: &BlockPtr,
        ) -> Result<(), StoreError> {
            match self {
                Storage::Shared => {
                    // error
                }

                Storage::Private(Schema { balance, .. }) => {
                    // use diesel::pg::upsert::excluded;
                    let query = format!(
                        "insert into {}(address, amount, block_number, block_hash) \
                     values ($1, $2, $3, $4) \
                         on conflict(address, block_number) \
                         do update set amount = $2",
                        balance.qname,
                    );

                    let bignum = graph::prelude::BigInt::from_unsigned_u256(&amount);
                    let slice = bignum.to_signed_bytes_le();
                    let nbigint = num_bigint::BigInt::from_signed_bytes_le(slice.as_slice());
                    let bigdecimal = graph::prelude::bigdecimal::BigDecimal::from(nbigint);
                    let num = diesel::pg::data_types::PgNumeric::from(bigdecimal);

                    sql_query(query)
                        .bind::<Bytea, _>(address.as_ref())
                        // need 256
                        .bind::<Numeric, _>(num)
                        .bind::<BigInt, _>(block_ptr.block_number() as i64)
                        .bind::<Bytea, _>(block_ptr.hash_slice())
                        .execute(conn)?;
                }
            };
            Ok(())
        }
        /// Insert a block. If the table already contains a block with the
        /// same hash, then overwrite that block since it may be adding
        /// transaction receipts.
        pub(super) fn upsert_block(
            &self,
            conn: &PgConnection,
            _chain: &str,
            block: EthereumBlock,
        ) -> Result<(), StoreError> {
            let number = block.block.number.unwrap().as_u64() as i64;
            let data = serde_json::to_value(&block).expect("Failed to serialize block");

            match self {
                Storage::Shared => {
                    // use public::ethereum_blocks as b;
                    use public::ethereum_transactions as t;

                    let _parent_hash = format!("{:x}", block.block.parent_hash);
                    let _hash = format!("{:x}", block.block.hash.unwrap());
                    // let values = (
                    //     b::hash.eq(hash),
                    //     b::number.eq(number),
                    //     b::parent_hash.eq(parent_hash),
                    //     b::network_name.eq(chain),
                    //     b::data.eq(data),
                    // );

                    let tx_values = block
                        .block
                        .transactions
                        .iter()
                        .map(|tx| {
                            let block_hash = format!("{:x}", block.block.hash.unwrap());
                            let block_number = number.clone();
                            let hash = format!("{:x}", tx.hash.clone());
                            let from = format!("{:x}", tx.from);
                            let value = format!("{:x}", tx.value);
                            let gas = format!("{:x}", tx.gas);
                            let gas_price = format!("{:x}", tx.gas_price);
                            let input = format!("{}", hex::encode(tx.input.0.clone()));
                            let nonce = format!("{:x}", tx.nonce.clone());
                            let transaction_index =
                                format!("{:x}", tx.transaction_index.unwrap().clone());
                            (
                                t::hash.eq(hash),
                                t::block_number.eq(block_number),
                                t::block_hash.eq(block_hash),
                                t::from.eq(from),
                                t::value.eq(value),
                                t::gas.eq(gas),
                                t::gas_price.eq(gas_price),
                                t::input.eq(input),
                                t::nonce.eq(nonce),
                                t::transaction_index.eq(transaction_index),
                            )
                        })
                        .collect::<Vec<_>>();

                    insert_into(t::table)
                        .values(tx_values)
                        .on_conflict(t::hash)
                        .do_nothing()
                        // .set((t::block_number.eq(excluded(t::block_number)), (t::block_hash.eq(excluded(t::block_hash)))))
                        .execute(conn)?;
                }

                Storage::Private(Schema {
                    blocks,
                    transactions,
                    receipts,
                    ..
                }) => {
                    // use diesel::pg::upsert::excluded;
                    let query = format!(
                        "insert into {}(hash, number, parent_hash, data) \
                     values ($1, $2, $3, $4) \
                         on conflict(hash) \
                         do update set number = $2, parent_hash = $3, data = $4",
                        blocks.qname,
                    );
                    let parent_hash = block.block.parent_hash;
                    let hash = block.block.hash.unwrap();

                    sql_query(query)
                        .bind::<Bytea, _>(hash.as_bytes())
                        .bind::<BigInt, _>(number)
                        .bind::<Bytea, _>(parent_hash.as_bytes())
                        .bind::<Jsonb, _>(data)
                        .execute(conn)?;

                    //
                    // receipts

                    let block_hash = BindSqlType::bytea(block.block.hash.unwrap());
                    for receipt in block.transaction_receipts.iter() {
                        //receipt sql insert
                        let cumulative_gas_used = format!("{}", receipt.cumulative_gas_used);
                        let effective_gas_used = format!("{}", receipt.effective_gas_used);

                        let gas_used = match receipt.gas_used {
                            Some(s) => format!("{}", s),
                            None => format!("null"),
                        };

                        let from = match receipt.from {
                            Some(s) => BindSqlType::bytea(s),
                            None => format!("null"),
                        };

                        let to = match receipt.to {
                            Some(s) => BindSqlType::bytea(s),
                            None => format!("null"),
                        };

                        let values = receipt
                            .logs
                            .iter()
                            .map(|log| {
                                let data = BindSqlType::bytea(log.data.clone());

                                let _topics = log
                                    .topics
                                    .iter()
                                    .enumerate()
                                    .map(|(_, i)| format!("{:x}", i))
                                    .collect::<Vec<String>>();
                                let topics = format!("'{{{}}}'", _topics.join(","));

                                let address = BindSqlType::bytea(log.address);
                                let log_type = match &log.log_type {
                                    Some(s) => format!("'{}'", s),
                                    None => format!("null"),
                                };

                                let removed = match log.removed {
                                    Some(s) => format!("{}", s),
                                    None => format!("null"),
                                };
                                let log_index = match log.log_index {
                                    Some(s) => BindSqlType::bytea(s),
                                    None => format!("null"),
                                };
                                let transaction_hash = match log.transaction_hash {
                                    Some(s) => BindSqlType::bytea(s),
                                    None => format!("null"),
                                };

                                let transaction_index = match log.transaction_index {
                                    Some(s) => BindSqlType::bytea(s),
                                    None => format!("null"),
                                };

                                format!(
                                    r#"({},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{})"#,
                                    transaction_hash,
                                    block_hash,
                                    number,
                                    data,
                                    topics,
                                    address,
                                    removed,
                                    log_index,
                                    log_type,
                                    transaction_hash,
                                    transaction_index,
                                    cumulative_gas_used,
                                    effective_gas_used,
                                    gas_used,
                                    from,
                                    to,
                                )
                            })
                            .collect::<Vec<_>>();

                        if values.len() == 0 {
                            continue;
                        }
                        let query = format!(
                            r#"insert into {}("id", "block_hash", "block_number", "data", "topics", "address", 
                                "removed", "log_index", "log_type", "transaction_hash", "transaction_index", 
                                "cumulative_gas_used", "effective_gas_used", "gas_used", "from", "to") 
                                values {} on conflict(id) do nothing"#,
                            receipts.qname,
                            values.join(","),
                        );
                        sql_query(&query).execute(conn).expect(&format!(
                            "Failed to insert {} ,sql:{}",
                            receipts.qname, &query
                        ));
                    }

                    // block transaction insert into db
                    if block.block.transactions.len() > 0 {
                        let tx_values = block
                            .block
                            .transactions
                            .iter()
                            .map(|tx| {
                                let block_hash = BindSqlType::bytea(block.block.hash.unwrap());
                                let block_number = number.clone();

                                let trx_type = tx.trx_type.as_u64() as i64;
                                let value = BindSqlType::bytea(tx.value);
                                let gas = tx.gas.as_u64() as i64;
                                let gas_price = tx.gas_price.as_u64() as i64;
                                let input = BindSqlType::bytea(tx.input.clone());
                                let nonce = BindSqlType::bytea(tx.nonce.clone());
                                let maxfeegas = match tx.max_fee_per_gas {
                                    Some(s) => format!("{}", s),
                                    None => format!("null"),
                                };
                                let maxprioritygas = match tx.max_priority_fee_per_gas {
                                    Some(s) => format!("{}", s),
                                    None => format!("null"),
                                };

                                let transaction_index =
                                    BindSqlType::bytea(tx.transaction_index.unwrap());

                                let hash = BindSqlType::bytea(tx.hash);
                                let from = BindSqlType::bytea(tx.from);
                                let to = match tx.to {
                                    Some(x) => BindSqlType::bytea(x),
                                    None => format!("null"),
                                };

                                format!(
                                    r#"({},{},{},{},{},{},{},{},{},{},{},{},{},{})"#,
                                    block_hash,
                                    block_number,
                                    hash,
                                    from,
                                    to,
                                    trx_type,
                                    value,
                                    gas,
                                    gas_price,
                                    input,
                                    nonce,
                                    transaction_index,
                                    maxfeegas,
                                    maxprioritygas
                                )
                            })
                            .collect::<Vec<_>>();

                        let query = format!(
                            "insert into {}(\"block_hash\", \"block_number\", \"hash\", \"from\", \"to\",\"trx_type\",\"value\", \"gas\", \"gas_price\", \"input\", \"nonce\", \"transaction_index\", \"max_fee_per_gas\", \"max_priority_fe_per_gas\") \
                            values {} on conflict(hash) do nothing",
                            transactions.qname,
                            tx_values.join(","),
                        );

                        sql_query(query)
                            .execute(conn)
                            .expect(&format!("Failed to insert {} data", transactions.qname));
                    }
                }
            };
            Ok(())
        }

        /// Insert a light block. On conflict do nothing, since we
        /// do not want to erase transaction receipts that might already
        /// be there
        pub(super) fn upsert_light_block(
            &self,
            conn: &PgConnection,
            chain: &str,
            block: LightEthereumBlock,
        ) -> Result<(), Error> {
            let hash = block.hash.unwrap();
            let parent_hash = block.parent_hash;
            let number = block.number.unwrap().as_u64() as i64;
            let data = serde_json::to_value(&EthereumBlock {
                block: Arc::new(block),
                transaction_receipts: Vec::new(),
            })
            .expect("Failed to serialize block");

            let result = match self {
                Storage::Shared => {
                    use public::ethereum_blocks as b;

                    let hash = format!("{:x}", hash);
                    let parent_hash = format!("{:x}", parent_hash);
                    let values = (
                        b::hash.eq(hash),
                        b::number.eq(number),
                        b::parent_hash.eq(parent_hash),
                        b::network_name.eq(chain),
                        b::data.eq(data),
                    );

                    insert_into(b::table)
                        .values(values.clone())
                        .on_conflict(b::hash)
                        .do_nothing()
                        .execute(conn)
                }
                Storage::Private(Schema { blocks, .. }) => {
                    let query = format!(
                        "insert into {}(hash, number, parent_hash, data) \
                         values ($1, $2, $3, $4) \
                             on conflict(hash) do nothing",
                        blocks.qname
                    );
                    sql_query(query)
                        .bind::<Bytea, _>(hash.as_bytes())
                        .bind::<BigInt, _>(number)
                        .bind::<Bytea, _>(parent_hash.as_bytes())
                        .bind::<Jsonb, _>(data)
                        .execute(conn)
                }
            };
            result.map(|_| ()).map_err(Error::from)
        }

        pub(super) fn blocks(
            &self,
            conn: &PgConnection,
            chain: &str,
            hashes: Vec<H256>,
        ) -> Result<Vec<LightEthereumBlock>, Error> {
            use diesel::dsl::any;

            let hashes = match self {
                Storage::Shared => {
                    use public::ethereum_blocks as b;

                    b::table
                        .select(sql::<Jsonb>("data -> 'block'"))
                        .filter(b::network_name.eq(chain))
                        .filter(b::hash.eq(any(Vec::from_iter(
                            hashes.into_iter().map(|h| format!("{:x}", h)),
                        ))))
                        .load::<serde_json::Value>(conn)?
                }
                Storage::Private(Schema { blocks, .. }) => blocks
                    .table()
                    .select(sql::<Jsonb>("data -> 'block'"))
                    .filter(
                        blocks
                            .hash()
                            .eq(any(Vec::from_iter(hashes.iter().map(|h| h.as_bytes())))),
                    )
                    .load::<serde_json::Value>(conn)?,
            };
            hashes
                .into_iter()
                .map(|block| serde_json::from_value(block).map_err(Into::into))
                .collect()
        }

        pub(super) fn block_hashes_by_block_number(
            &self,
            conn: &PgConnection,
            chain: &str,
            number: BlockNumber,
        ) -> Result<Vec<H256>, Error> {
            match self {
                Storage::Shared => {
                    use public::ethereum_blocks as b;

                    b::table
                        .select(b::hash)
                        .filter(b::network_name.eq(&chain))
                        .filter(b::number.eq(number as i64))
                        .get_results::<String>(conn)?
                        .into_iter()
                        .map(|h| h.parse())
                        .collect::<Result<Vec<H256>, _>>()
                        .map_err(Error::from)
                }
                Storage::Private(Schema { blocks, .. }) => blocks
                    .table()
                    .select(blocks.hash())
                    .filter(blocks.number().eq(number as i64))
                    .get_results::<Vec<u8>>(conn)?
                    .into_iter()
                    .map(|hash| h256_from_bytes(hash.as_slice()))
                    .collect::<Result<Vec<H256>, _>>()
                    .map_err(Error::from),
            }
        }

        pub(super) fn confirm_block_hash(
            &self,
            conn: &PgConnection,
            chain: &str,
            number: BlockNumber,
            hash: &H256,
        ) -> Result<usize, Error> {
            let number = number as i64;

            match self {
                Storage::Shared => {
                    use public::ethereum_blocks as b;

                    let hash = format!("{:x}", hash);
                    diesel::delete(b::table)
                        .filter(b::network_name.eq(chain))
                        .filter(b::number.eq(number))
                        .filter(b::hash.ne(&hash))
                        .execute(conn)
                        .map_err(Error::from)
                }
                Storage::Private(Schema { blocks, .. }) => {
                    let query = format!(
                        "delete from {} where number = $1 and hash != $2",
                        blocks.qname
                    );
                    sql_query(query)
                        .bind::<BigInt, _>(number)
                        .bind::<Bytea, _>(hash.as_bytes())
                        .execute(conn)
                        .map_err(Error::from)
                }
            }
        }

        pub(super) fn block_number(
            &self,
            conn: &PgConnection,
            hash: H256,
        ) -> Result<Option<BlockNumber>, StoreError> {
            let number = match self {
                Storage::Shared => {
                    use public::ethereum_blocks as b;

                    b::table
                        .select(b::number)
                        .filter(b::hash.eq(format!("{:x}", hash)))
                        .first::<i64>(conn)
                        .optional()?
                }
                Storage::Private(Schema { blocks, .. }) => blocks
                    .table()
                    .select(blocks.number())
                    .filter(blocks.hash().eq(hash.as_bytes()))
                    .first::<i64>(conn)
                    .optional()?,
            };
            number
                .map(|number| {
                    BlockNumber::try_from(number)
                        .map_err(|e| StoreError::QueryExecutionError(e.to_string()))
                })
                .transpose()
        }

        /// Find the first block that is missing from the database needed to
        /// complete the chain from block `hash` to the block with number
        /// `first_block`.
        pub(super) fn missing_parent(
            &self,
            conn: &PgConnection,
            chain: &str,
            first_block: i64,
            hash: H256,
            genesis: H256,
        ) -> Result<Option<H256>, Error> {
            match self {
                Storage::Shared => {
                    // We recursively build a temp table 'chain' containing the hash and
                    // parent_hash of blocks to check. The 'last' value is used to stop
                    // the recursion and is true if one of these conditions is true:
                    //   * we are missing a parent block
                    //   * we checked the required number of blocks
                    //   * we checked the genesis block
                    const MISSING_PARENT_SQL: &str = "
            with recursive chain(hash, parent_hash, last) as (
                -- base case: look at the head candidate block
                select b.hash, b.parent_hash, false
                  from ethereum_blocks b
                 where b.network_name = $1
                   and b.hash = $2
                   and b.hash != $3
                union all
                -- recursion step: add a block whose hash is the latest parent_hash
                -- on chain
                select chain.parent_hash,
                       b.parent_hash,
                       coalesce(b.parent_hash is null
                             or b.number <= $4
                             or b.hash = $3, true)
                  from chain left outer join ethereum_blocks b
                              on chain.parent_hash = b.hash
                             and b.network_name = $1
                 where not chain.last)
             select hash
               from chain
              where chain.parent_hash is null;
            ";

                    let hash = format!("{:x}", hash);
                    let genesis = format!("{:x}", genesis);
                    let missing = sql_query(MISSING_PARENT_SQL)
                        .bind::<Text, _>(chain)
                        .bind::<Text, _>(&hash)
                        .bind::<Text, _>(&genesis)
                        .bind::<BigInt, _>(first_block)
                        .load::<BlockHashText>(conn)?;

                    let missing = match missing.len() {
                        0 => None,
                        1 => Some(missing[0].hash.parse()?),
                        _ => {
                            unreachable!("the query can only return no or one row");
                        }
                    };
                    Ok(missing)
                }
                Storage::Private(Schema { blocks, .. }) => {
                    // This is the same as `MISSING_PARENT_SQL` above except that
                    // the blocks table has a different name and that it does
                    // not have a `network_name` column
                    let query = format!(
                        "
            with recursive chain(hash, parent_hash, last) as (
                -- base case: look at the head candidate block
                select b.hash, b.parent_hash, false
                  from {qname} b
                 where b.hash = $1
                   and b.hash != $2
                union all
                -- recursion step: add a block whose hash is the latest parent_hash
                -- on chain
                select chain.parent_hash,
                       b.parent_hash,
                       coalesce(b.parent_hash is null
                             or b.number <= $3
                             or b.hash = $2, true)
                  from chain left outer join {qname} b
                              on chain.parent_hash = b.hash
                 where not chain.last)
             select hash
               from chain
              where chain.parent_hash is null;
            ",
                        qname = blocks.qname
                    );

                    let missing = sql_query(query)
                        .bind::<Bytea, _>(hash.as_bytes())
                        .bind::<Bytea, _>(genesis.as_bytes())
                        .bind::<BigInt, _>(first_block)
                        .load::<BlockHashBytea>(conn)?;

                    let missing = match missing.len() {
                        0 => None,
                        1 => Some(h256_from_bytes(&missing[0].hash)?),
                        _ => {
                            unreachable!("the query can only return no or one row")
                        }
                    };
                    Ok(missing)
                }
            }
        }

        /// Return the best candidate for the new chain head if there is a block
        /// with a higher block number than the current chain head. The returned
        /// value if the hash and number of the candidate and the genesis block
        /// hash for the chain
        pub(super) fn chain_head_candidate(
            &self,
            conn: &PgConnection,
            chain: &str,
        ) -> Result<Option<BlockPtr>, Error> {
            use public::ethereum_networks as n;

            let head = n::table
                .filter(n::name.eq(chain))
                .select(n::head_block_number)
                .first::<Option<i64>>(conn)?
                .unwrap_or(-1);

            match self {
                Storage::Shared => {
                    use public::ethereum_blocks as b;
                    b::table
                        .filter(b::network_name.eq(chain))
                        .filter(b::number.gt(head))
                        .order_by((b::number.desc(), b::hash))
                        .select((b::hash, b::number))
                        .first::<(String, i64)>(conn)
                        .optional()?
                        .map(|(hash, number)| BlockPtr::try_from((hash.as_str(), number)))
                        .transpose()
                }
                Storage::Private(Schema { blocks, .. }) => blocks
                    .table()
                    .filter(blocks.number().gt(head))
                    .order_by((blocks.number().desc(), blocks.hash()))
                    .select((blocks.hash(), blocks.number()))
                    .first::<(Vec<u8>, i64)>(conn)
                    .optional()?
                    .map(|(hash, number)| BlockPtr::try_from((hash.as_slice(), number)))
                    .transpose(),
            }
        }

        // pub(super) fn chain_early_head_candidate(
        //     &self,
        //     conn: &PgConnection,
        //     chain: &str,
        // ) -> Result<Option<BlockPtr>, Error> {
        //     use public::ethereum_networks as n;

        //     let (head_num, head_hash) = n::table
        //         .filter(n::name.eq(chain))
        //         .select((n::early_head_block_number, n::early_head_block_hash))
        //         .first::<(Option<i64>, Option<String>)>(conn)
        //         .optional()?
        //         .map(|(num, hash)| (num.unwrap_or(i64::MAX), hash.unwrap_or("".to_string())))
        //         .unwrap();

        //     match self {
        //         Storage::Shared => {
        //             use public::ethereum_blocks as b;
        //             b::table
        //                 .filter(b::network_name.eq(chain))
        //                 .filter(b::number.lt(head_num))
        //                 .order_by((b::number.desc(), b::hash))
        //                 .select((b::hash, b::number))
        //                 .first::<(String, i64)>(conn)
        //                 .optional()?
        //                 .map(|(hash, number)| BlockPtr::try_from((hash.as_str(), number)))
        //                 .transpose()
        //         }
        //         Storage::Private(Schema { blocks, .. }) => blocks
        //             .table()
        //             .filter(blocks.number().lt(head_num))
        //             .order_by((blocks.number().desc(), blocks.hash()))
        //             .select((blocks.hash(), blocks.number()))
        //             .first::<(Vec<u8>, i64)>(conn)
        //             .optional()?
        //             .map(|(hash, number)| BlockPtr::try_from((hash.as_slice(), number)))
        //             .transpose(),
        //     }
        // }

        pub(super) fn ancestor_block(
            &self,
            conn: &PgConnection,
            block_ptr: BlockPtr,
            offset: BlockNumber,
        ) -> Result<Option<EthereumBlock>, Error> {
            let data = match self {
                Storage::Shared => {
                    const ANCESTOR_SQL: &str = "
        with recursive ancestors(block_hash, block_offset) as (
            values ($1, 0)
            union all
            select b.parent_hash, a.block_offset+1
              from ancestors a, ethereum_blocks b
             where a.block_hash = b.hash
               and a.block_offset < $2
        )
        select a.block_hash as hash
          from ancestors a
         where a.block_offset = $2;";

                    let hash = sql_query(ANCESTOR_SQL)
                        .bind::<Text, _>(block_ptr.hash_hex())
                        .bind::<BigInt, _>(offset as i64)
                        .get_result::<BlockHashText>(conn)
                        .optional()?;

                    use public::ethereum_blocks as b;

                    match hash {
                        None => None,
                        Some(hash) => Some(
                            b::table
                                .filter(b::hash.eq(hash.hash))
                                .select(b::data)
                                .first::<serde_json::Value>(conn)?,
                        ),
                    }
                }
                Storage::Private(Schema { blocks, .. }) => {
                    // Same as ANCESTOR_SQL except for the table name
                    let query = format!(
                        "
        with recursive ancestors(block_hash, block_offset) as (
            values ($1, 0)
            union all
            select b.parent_hash, a.block_offset+1
              from ancestors a, {} b
             where a.block_hash = b.hash
               and a.block_offset < $2
        )
        select a.block_hash as hash
          from ancestors a
         where a.block_offset = $2;",
                        blocks.qname
                    );

                    let hash = sql_query(query)
                        .bind::<Bytea, _>(block_ptr.hash_slice())
                        .bind::<BigInt, _>(offset as i64)
                        .get_result::<BlockHashBytea>(conn)
                        .optional()?;
                    match hash {
                        None => None,
                        Some(hash) => Some(
                            blocks
                                .table()
                                .filter(blocks.hash().eq(hash.hash))
                                .select(blocks.data())
                                .first::<serde_json::Value>(conn)?,
                        ),
                    }
                }
            };

            let block = data
                .map(|data| serde_json::from_value::<EthereumBlock>(data))
                .transpose()
                .expect("Failed to deserialize block from database");

            Ok(block)
        }

        pub(super) fn delete_blocks_before(
            &self,
            conn: &PgConnection,
            chain: &str,
            block: i64,
        ) -> Result<usize, Error> {
            match self {
                Storage::Shared => {
                    use public::ethereum_blocks as b;

                    diesel::delete(b::table)
                        .filter(b::network_name.eq(chain))
                        .filter(b::number.lt(block))
                        .filter(b::number.gt(0))
                        .execute(conn)
                        .map_err(Error::from)
                }
                Storage::Private(Schema { blocks, .. }) => {
                    let query = format!(
                        "delete from {} where number < $1 and number > 0",
                        blocks.qname
                    );
                    sql_query(query)
                        .bind::<BigInt, _>(block)
                        .execute(conn)
                        .map_err(Error::from)
                }
            }
        }

        pub(super) fn get_call_and_access(
            &self,
            conn: &PgConnection,
            id: &[u8],
        ) -> Result<Option<(Vec<u8>, bool)>, Error> {
            match self {
                Storage::Shared => {
                    use public::eth_call_cache as cache;
                    use public::eth_call_meta as meta;

                    cache::table
                        .find(id.as_ref())
                        .inner_join(meta::table)
                        .select((
                            cache::return_value,
                            sql("CURRENT_DATE > eth_call_meta.accessed_at"),
                        ))
                        .get_result(conn)
                        .optional()
                        .map_err(Error::from)
                }
                Storage::Private(Schema {
                    call_cache,
                    call_meta,
                    ..
                }) => call_cache
                    .table()
                    .inner_join(
                        call_meta.table().on(call_meta
                            .contract_address()
                            .eq(call_cache.contract_address())),
                    )
                    .filter(call_cache.id().eq(id))
                    .select((
                        call_cache.return_value(),
                        sql(&format!(
                            "CURRENT_DATE > {}.{}",
                            CallMetaTable::TABLE_NAME,
                            CallMetaTable::ACCESSED_AT
                        )),
                    ))
                    .first(conn)
                    .optional()
                    .map_err(Error::from),
            }
        }

        pub(super) fn update_accessed_at(
            &self,
            conn: &PgConnection,
            contract_address: &[u8],
        ) -> Result<(), Error> {
            let result = match self {
                Storage::Shared => {
                    use public::eth_call_meta as meta;

                    update(meta::table.find(contract_address.as_ref()))
                        .set(meta::accessed_at.eq(sql("CURRENT_DATE")))
                        .execute(conn)
                }
                Storage::Private(Schema { call_meta, .. }) => {
                    let query = format!(
                        "update {} set accessed_at = CURRENT_DATE where contract_address = $1",
                        call_meta.qname
                    );
                    sql_query(query)
                        .bind::<Bytea, _>(contract_address)
                        .execute(conn)
                }
            };
            result.map(|_| ()).map_err(Error::from)
        }

        pub(super) fn set_call(
            &self,
            conn: &PgConnection,
            id: &[u8],
            contract_address: &[u8],
            block_number: i32,
            return_value: &[u8],
            method_id: &[u8],
            call_args: Vec<String>,
        ) -> Result<(), Error> {
            let result = match self {
                Storage::Shared => {
                    use public::eth_call_cache as cache;
                    use public::eth_call_meta as meta;

                    insert_into(cache::table)
                        .values((
                            cache::id.eq(id),
                            cache::contract_address.eq(contract_address),
                            cache::block_number.eq(block_number),
                            cache::return_value.eq(return_value),
                        ))
                        .on_conflict_do_nothing()
                        .execute(conn)?;

                    let accessed_at = meta::accessed_at.eq(sql("CURRENT_DATE"));
                    insert_into(meta::table)
                        .values((
                            meta::contract_address.eq(contract_address.as_ref()),
                            accessed_at.clone(),
                        ))
                        .on_conflict(meta::contract_address)
                        .do_update()
                        .set(accessed_at)
                        // TODO: Add a where clause similar to the Private
                        // branch to avoid unnecessary updates (not entirely
                        // trivial with diesel)
                        .execute(conn)
                }
                Storage::Private(Schema {
                    call_cache,
                    call_meta,
                    ..
                }) => {
                    let query = format!(
                        "insert into {}(id, contract_address, block_number, return_value, method_id, method_params) \
                         values ($1, $2, $3, $4, $5, $6) on conflict do nothing",
                        call_cache.qname
                    );

                    sql_query(query)
                        .bind::<Bytea, _>(id)
                        .bind::<Bytea, _>(contract_address)
                        .bind::<Integer, _>(block_number)
                        .bind::<Bytea, _>(return_value)
                        .bind::<Bytea, _>(&method_id)
                        .bind::<Nullable<Text>, _>(if call_args.len() == 0 {
                            None
                        } else {
                            Some(call_args.join(","))
                        })
                        .execute(conn)?;
                    let query = format!(
                        "insert into {}(contract_address, accessed_at) \
                         values ($1, CURRENT_DATE) \
                         on conflict(contract_address)
                         do update set accessed_at = CURRENT_DATE \
                                 where excluded.accessed_at < CURRENT_DATE",
                        call_meta.qname
                    );
                    sql_query(query)
                        .bind::<Bytea, _>(contract_address)
                        .execute(conn)
                }
            };
            result.map(|_| ()).map_err(Error::from)
        }

        #[cfg(debug_assertions)]
        // used by `super::set_chain` for test support
        pub(super) fn set_chain(
            &self,
            conn: &PgConnection,
            chain_name: &str,
            genesis_hash: &str,
            chain: super::test_support::Chain,
        ) {
            use public::ethereum_networks as n;

            match self {
                Storage::Shared => {
                    use public::eth_call_cache as c;
                    use public::eth_call_meta as m;
                    use public::ethereum_blocks as b;

                    diesel::delete(b::table.filter(b::network_name.eq(chain_name)))
                        .execute(conn)
                        .expect("Failed to delete ethereum_blocks");
                    // We don't have a good way to clean out the call cache
                    // per chain; just nuke everything
                    diesel::delete(c::table).execute(conn).unwrap();
                    diesel::delete(m::table).execute(conn).unwrap();
                }
                Storage::Private(Schema {
                    blocks,
                    call_meta,
                    call_cache,
                    ..
                }) => {
                    for qname in &[&blocks.qname, &call_meta.qname, &call_cache.qname] {
                        let query = format!("delete from {}", qname);
                        sql_query(query)
                            .execute(conn)
                            .expect(&format!("Failed to delete {}", qname));
                    }
                }
            }

            for block in &chain {
                self.upsert_block(conn, chain_name, block.as_ethereum_block())
                    .unwrap();
            }

            diesel::update(n::table.filter(n::name.eq(chain_name)))
                .set((
                    n::genesis_block_hash.eq(genesis_hash),
                    n::head_block_hash.eq::<Option<&str>>(None),
                    n::head_block_number.eq::<Option<i64>>(None),
                    n::early_head_block_hash.eq::<Option<&str>>(None),
                    n::early_head_block_number.eq::<Option<i64>>(None),
                ))
                .execute(conn)
                .unwrap();
        }

        /// Queries the database for all the transaction receipts in a given block range.
        pub(crate) fn find_transaction_receipts_in_block(
            &self,
            conn: &PgConnection,
            block_hash: H256,
        ) -> anyhow::Result<Vec<LightTransactionReceipt>> {
            let query = sql_query(format!(
                "
select
    ethereum_hex_to_bytea(receipt ->> 'transactionHash') as transaction_hash,
    ethereum_hex_to_bytea(receipt ->> 'transactionIndex') as transaction_index,
    ethereum_hex_to_bytea(receipt ->> 'blockHash') as block_hash,
    ethereum_hex_to_bytea(receipt ->> 'blockNumber') as block_number,
    ethereum_hex_to_bytea(receipt ->> 'gasUsed') as gas_used,
    ethereum_hex_to_bytea(receipt ->> 'status') as status
from (
    select
        jsonb_array_elements(data -> 'transaction_receipts') as receipt
    from
        {blocks_table_name}
    where hash = $1) as temp;
",
                blocks_table_name = self.blocks_table()
            ));

            let query_results: Result<Vec<RawTransactionReceipt>, diesel::result::Error> = {
                // The `hash` column has different types between the `public.ethereum_blocks` and the
                // `chain*.blocks` tables, so we must check which one is being queried to bind the
                // `block_hash` parameter to the correct type
                match self {
                    Storage::Shared => query
                        .bind::<Text, _>(format!("{:x}", block_hash))
                        .get_results(conn),
                    Storage::Private(_) => query
                        .bind::<Binary, _>(block_hash.as_bytes())
                        .get_results(conn),
                }
            };
            query_results
                .map_err(|error| {
                    anyhow::anyhow!(
                        "Error fetching transaction receipt from database: {}",
                        error
                    )
                })?
                .into_iter()
                .map(LightTransactionReceipt::try_from)
                .collect()
        }

        // for balance
        pub(crate) fn find_transaction_address(
            &self,
            conn: &PgConnection,
            block_ptr: &BlockPtr,
        ) -> anyhow::Result<Vec<web3::types::Address>, Error> {
            match self {
                Storage::Shared => Ok(Vec::<web3::types::Address>::new()),
                Storage::Private(Schema { transactions, .. }) => {
                    let rst = transactions
                        .table()
                        .select(transactions.to())
                        .filter(transactions.block_number().eq(block_ptr.number as i64))
                        .filter(transactions.to().is_not_null())
                        .get_results::<Vec<u8>>(conn)?;

                    let ret = rst
                        .into_iter()
                        .map(|addr| H160::from_slice(addr.as_slice()))
                        .collect::<Vec<H160>>();
                    Ok(ret)
                }
            }
        }
    }
}

pub struct ChainStore {
    pool: ConnectionPool,
    pub chain: String,
    pub(crate) storage: data::Storage,
    genesis_block_ptr: BlockPtr,
    status: ChainStatus,
    chain_head_update_sender: ChainHeadUpdateSender,
}

impl ChainStore {
    pub(crate) fn new(
        chain: String,
        storage: data::Storage,
        net_identifier: &EthereumNetworkIdentifier,
        status: ChainStatus,
        chain_head_update_sender: ChainHeadUpdateSender,
        pool: ConnectionPool,
    ) -> Self {
        let store = ChainStore {
            pool,
            chain,
            storage,
            genesis_block_ptr: (net_identifier.genesis_block_hash, 0 as u64).into(),
            status,
            chain_head_update_sender,
        };

        store
    }

    pub fn is_ingestible(&self) -> bool {
        matches!(self.status, ChainStatus::Ingestible)
    }

    fn get_conn(&self) -> Result<PooledConnection<ConnectionManager<PgConnection>>, Error> {
        self.pool.get().map_err(Error::from)
    }

    pub(crate) fn create(&self, ident: &EthereumNetworkIdentifier) -> Result<(), Error> {
        {
            use public::ethereum_networks::dsl::*;

            let conn = self.get_conn()?;
            conn.transaction(|| {
                insert_into(ethereum_networks)
                    .values((
                        name.eq(&self.chain),
                        namespace.eq(&self.storage),
                        head_block_hash.eq::<Option<String>>(None),
                        head_block_number.eq::<Option<i64>>(None),
                        early_head_block_hash.eq::<Option<String>>(None),
                        early_head_block_number.eq::<Option<i64>>(None),
                        net_version.eq(&ident.net_version),
                        genesis_block_hash.eq(format!("{:x}", ident.genesis_block_hash)),
                    ))
                    .on_conflict(name)
                    .do_nothing()
                    .execute(&conn)?;
                self.storage.create(&conn)
            })?;
        }
        // balnace
        {
            use public::ethereum_balance as eb;
            use public::ethereum_balance::dsl::*;
            let conn = self.get_conn()?;

            conn.transaction(|| {
                insert_into(eb::table)
                    .values((
                        name.eq(&self.chain),
                        namespace.eq(&self.storage),
                        head_block_hash.eq::<Option<String>>(None),
                        head_block_number.eq::<Option<i64>>(None),
                        early_head_block_hash.eq::<Option<String>>(None),
                        early_head_block_number.eq::<Option<i64>>(None),
                        net_version.eq(&ident.net_version),
                        genesis_block_hash.eq(format!("{:x}", ident.genesis_block_hash)),
                    ))
                    .on_conflict(name)
                    .do_nothing()
                    .execute(&conn)
            })?;
        }
        Ok(())
    }

    pub(crate) fn drop_chain(&self) -> Result<(), Error> {
        use diesel::dsl::delete;
        use public::ethereum_networks as n;

        let conn = self.get_conn()?;
        conn.transaction(|| {
            self.storage.drop_storage(&conn, &self.chain)?;

            delete(n::table.filter(n::name.eq(&self.chain))).execute(&conn)?;
            Ok(())
        })
    }

    pub fn chain_head_pointers(&self) -> Result<HashMap<String, BlockPtr>, StoreError> {
        use public::ethereum_networks as n;

        let pointers: Vec<(String, BlockPtr)> = n::table
            .select((n::name, n::head_block_hash, n::head_block_number))
            .load::<(String, Option<String>, Option<i64>)>(&self.get_conn()?)?
            .into_iter()
            .filter_map(|(name, hash, number)| match (hash, number) {
                (Some(hash), Some(number)) => Some((name, hash, number)),
                _ => None,
            })
            .map(|(name, hash, number)| {
                BlockPtr::try_from((hash.as_str(), number)).map(|ptr| (name, ptr))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(HashMap::from_iter(pointers))
    }

    pub fn chain_head_block(&self, chain: &str) -> Result<Option<BlockNumber>, StoreError> {
        use public::ethereum_networks as n;

        let number: Option<i64> = n::table
            .filter(n::name.eq(chain))
            .select(n::head_block_number)
            .first::<Option<i64>>(&self.get_conn()?)
            .optional()?
            .flatten();

        number.map(|number| number.try_into()).transpose().map_err(
            |e: std::num::TryFromIntError| {
                constraint_violation!(
                    "head block number for {} is {:?} which does not fit into a u32: {}",
                    chain,
                    number,
                    e.to_string()
                )
            },
        )
    }
}

#[async_trait]
impl ChainStoreTrait for ChainStore {
    fn genesis_block_ptr(&self) -> Result<BlockPtr, Error> {
        Ok(self.genesis_block_ptr.clone())
    }

    async fn upsert_balance(
        &self,
        address: &Address,
        amount: U256,
        block_ptr: &BlockPtr,
    ) -> Result<(), Error> {
        let pool = self.pool.clone();
        let address_clone = address.clone();
        let block_ptr_clone = block_ptr.clone();
        let storage = self.storage.clone();
        pool.with_conn(move |conn, _| {
            conn.transaction(|| {
                storage
                    .upsert_balance(&conn, &address_clone, amount, &block_ptr_clone)
                    .map_err(CancelableError::from)
            })
        })
        .await
        .map_err(Error::from)
    }

    async fn upsert_block(&self, block: EthereumBlock) -> Result<(), Error> {
        let pool = self.pool.clone();
        let network = self.chain.clone();
        let storage = self.storage.clone();
        pool.with_conn(move |conn, _| {
            conn.transaction(|| {
                storage
                    .upsert_block(&conn, &network, block)
                    .map_err(CancelableError::from)
            })
        })
        .await
        .map_err(Error::from)
    }

    fn upsert_light_blocks(&self, blocks: Vec<LightEthereumBlock>) -> Result<(), Error> {
        let conn = self.pool.get()?;
        for block in blocks {
            self.storage.upsert_light_block(&conn, &self.chain, block)?;
        }
        Ok(())
    }

    async fn early_attempt_chain_head_update(
        self: Arc<Self>,
        parent_num: BlockNumber,
        parent_hash: H256,
    ) -> Result<(), Error> {
        use public::ethereum_networks as n;
        let chain_store = self.clone();
        let _ret = self
            .pool
            .with_conn(move |conn, _| {
                let hash = format!("{:x}", parent_hash);
                let number = parent_num as i64;

                conn.transaction(|| -> Result<(), StoreError> {
                    update(n::table.filter(n::name.eq(&chain_store.chain)))
                        .set((
                            n::early_head_block_hash.eq(&hash),
                            n::early_head_block_number.eq(number),
                            n::early_head_updated.eq(diesel::dsl::now),
                        ))
                        .execute(conn)?;
                    Ok(())
                })
                .map_err(CancelableError::from)
            })
            .await?;
        Ok(())
    }

    async fn attempt_chain_head_update(
        self: Arc<Self>,
        ancestor_count: BlockNumber,
    ) -> Result<Option<H256>, Error> {
        use public::ethereum_networks as n;

        let (missing, ptr) = {
            let chain_store = self.clone();
            self.pool
                .with_conn(move |conn, _| {
                    let candidate = chain_store
                        .storage
                        .chain_head_candidate(&conn, &chain_store.chain)
                        .map_err(CancelableError::from)?;
                    let (ptr, first_block) = match &candidate {
                        None => return Ok((None, None)),
                        Some(ptr) => (ptr, 0.max(ptr.number.saturating_sub(ancestor_count))),
                    };

                    match chain_store
                        .storage
                        .missing_parent(
                            &conn,
                            &chain_store.chain,
                            first_block as i64,
                            ptr.hash_as_h256(),
                            chain_store.genesis_block_ptr.hash_as_h256(),
                        )
                        .map_err(CancelableError::from)?
                    {
                        Some(missing) => {
                            return Ok((Some(missing), None));
                        }
                        None => { /* we have a complete chain, no missing parents */ }
                    }

                    let hash = ptr.hash_hex();
                    let number = ptr.number as i64;

                    conn.transaction(
                        || -> Result<(Option<H256>, Option<(String, i64)>), StoreError> {
                            update(n::table.filter(n::name.eq(&chain_store.chain)))
                                .set((
                                    n::head_block_hash.eq(&hash),
                                    n::head_block_number.eq(number),
                                    n::head_updated.eq(diesel::dsl::now),
                                ))
                                .execute(conn)?;
                            Ok((None, Some((hash, number))))
                        },
                    )
                    .map_err(CancelableError::from)
                })
                .await?
        };
        if let Some((hash, number)) = ptr {
            self.chain_head_update_sender.send(&hash, number)?;
        }

        Ok(missing)
    }

    fn chain_early_head_ptr(&self) -> Result<Option<BlockPtr>, Error> {
        use public::ethereum_networks::dsl::*;

        ethereum_networks
            .select((early_head_block_hash, early_head_block_number))
            .filter(name.eq(&self.chain))
            .load::<(Option<String>, Option<i64>)>(&*self.get_conn()?)
            .map(|rows| {
                rows.first()
                    .map(|(hash_opt, number_opt)| match (hash_opt, number_opt) {
                        (Some(hash), Some(number)) => Some((hash.parse().unwrap(), *number).into()),
                        (None, None) => None,
                        _ => unreachable!(),
                    })
                    .and_then(|opt| opt)
            })
            .map_err(Error::from)
    }

    fn chain_head_ptr(&self) -> Result<Option<BlockPtr>, Error> {
        use public::ethereum_networks::dsl::*;

        ethereum_networks
            .select((head_block_hash, head_block_number))
            .filter(name.eq(&self.chain))
            .load::<(Option<String>, Option<i64>)>(&*self.get_conn()?)
            .map(|rows| {
                rows.first()
                    .map(|(hash_opt, number_opt)| match (hash_opt, number_opt) {
                        (Some(hash), Some(number)) => Some((hash.parse().unwrap(), *number).into()),
                        (None, None) => None,
                        _ => unreachable!(),
                    })
                    .and_then(|opt| opt)
            })
            .map_err(Error::from)
    }

    fn blocks(&self, hashes: Vec<H256>) -> Result<Vec<LightEthereumBlock>, Error> {
        let conn = self.get_conn()?;
        self.storage.blocks(&conn, &self.chain, hashes)
    }

    fn ancestor_block(
        &self,
        block_ptr: BlockPtr,
        offset: BlockNumber,
    ) -> Result<Option<EthereumBlock>, Error> {
        ensure!(
            block_ptr.number >= offset,
            "block offset {} for block `{}` points to before genesis block",
            offset,
            block_ptr.hash_hex()
        );

        let conn = self.get_conn()?;
        self.storage.ancestor_block(&conn, block_ptr, offset)
    }

    fn cleanup_cached_blocks(
        &self,
        ancestor_count: BlockNumber,
    ) -> Result<Option<(BlockNumber, usize)>, Error> {
        use diesel::sql_types::Integer;

        #[derive(QueryableByName)]
        struct MinBlock {
            #[sql_type = "Integer"]
            block: i32,
        }

        // Remove all blocks from the cache that are behind the slowest
        // subgraph's head block, but retain the genesis block. We stay
        // behind the slowest subgraph so that we do not interfere with its
        // syncing activity.
        // We also stay `ancestor_count` many blocks behind the head of the
        // chain since the block ingestor consults these blocks frequently
        //
        // Only consider active subgraphs that have not failed

        // This assumes that subgraph metadata and blocks are stored in the
        // same shard. We disallow setting GRAPH_ETHEREUM_CLEANUP_BLOCKS in
        // graph_node::config so that we only run this query when we know
        // it will work. Running this with a sharded store might remove
        // blocks that are still needed by deployments in other shard
        //
        // See 8b6ad0c64e244023ac20ced7897fe666

        let conn = self.get_conn()?;
        let query = "
            select coalesce(
                   least(a.block,
                        (select head_block_number::int - $1
                           from ethereum_networks
                          where name = $2)), -1)::int as block
              from (
                select min(d.latest_ethereum_block_number) as block
                  from subgraphs.subgraph_deployment d,
                       subgraphs.subgraph_deployment_assignment a,
                       deployment_schemas ds
                 where ds.subgraph = d.deployment
                   and a.id = d.id
                   and not d.failed
                   and ds.network = $2) a;";
        let ancestor_count = i32::try_from(ancestor_count)
            .expect("ancestor_count fits into a signed 32 bit integer");
        diesel::sql_query(query)
            .bind::<Integer, _>(ancestor_count)
            .bind::<Text, _>(&self.chain)
            .load::<MinBlock>(&conn)?
            .first()
            .map(|MinBlock { block }| {
                // If we could not determine a minimum block, the query
                // returns -1, and we should not do anything. We also guard
                // against removing the genesis block
                if *block > 0 {
                    self.storage
                        .delete_blocks_before(&conn, &self.chain, *block as i64)
                        .map(|rows| Some((*block, rows)))
                } else {
                    Ok(None)
                }
            })
            .unwrap_or(Ok(None))
            .map_err(|e| e.into())
    }

    fn block_hashes_by_block_number(&self, number: BlockNumber) -> Result<Vec<H256>, Error> {
        let conn = self.get_conn()?;
        self.storage
            .block_hashes_by_block_number(&conn, &self.chain, number)
    }

    fn confirm_block_hash(&self, number: BlockNumber, hash: &H256) -> Result<usize, Error> {
        let conn = self.get_conn()?;
        self.storage
            .confirm_block_hash(&conn, &self.chain, number, hash)
    }

    fn block_number(&self, hash: H256) -> Result<Option<(String, BlockNumber)>, StoreError> {
        let conn = self.get_conn()?;
        Ok(self
            .storage
            .block_number(&conn, hash)?
            .map(|number| (self.chain.clone(), number)))
    }

    async fn transaction_receipts_in_block(
        &self,
        block_hash: &H256,
    ) -> Result<Vec<LightTransactionReceipt>, StoreError> {
        let pool = self.pool.clone();
        let storage = self.storage.clone();
        let block_hash = block_hash.to_owned();
        pool.with_conn(move |conn, _| {
            storage
                .find_transaction_receipts_in_block(&conn, block_hash)
                .map_err(|e| StoreError::from(e).into())
        })
        .await
    }

    /// for balance
    fn chain_balance_head_ptr(&self) -> Result<Option<BlockPtr>, Error> {
        use public::ethereum_balance as eb;

        eb::table
            .select((eb::head_block_hash, eb::head_block_number))
            .filter(eb::name.eq(&self.chain))
            .load::<(Option<String>, Option<i64>)>(&*self.get_conn()?)
            .map(|rows| {
                rows.first()
                    .map(|(hash_opt, number_opt)| match (hash_opt, number_opt) {
                        (Some(hash), Some(number)) => Some((hash.parse().unwrap(), *number).into()),
                        (None, None) => None,
                        _ => unreachable!(),
                    })
                    .and_then(|opt| opt)
            })
            .map_err(Error::from)
    }
    fn chain_balance_early_head_ptr(&self) -> Result<Option<BlockPtr>, Error> {
        use public::ethereum_balance as eb;

        eb::table
            .select((eb::early_head_block_hash, eb::early_head_block_number))
            .filter(eb::name.eq(&self.chain))
            .load::<(Option<String>, Option<i64>)>(&*self.get_conn()?)
            .map(|rows| {
                rows.first()
                    .map(|(hash_opt, number_opt)| match (hash_opt, number_opt) {
                        (Some(hash), Some(number)) => Some((hash.parse().unwrap(), *number).into()),
                        (None, None) => None,
                        _ => unreachable!(),
                    })
                    .and_then(|opt| opt)
            })
            .map_err(Error::from)
    }

    async fn chain_update_balance_head(&self, block_ptr: &BlockPtr) -> Result<u64, Error> {
        use public::ethereum_balance as n;

        let chain_store = self.clone();
        let conn = self.pool.get().expect("can get a database connection");

        let ret = update(n::table.filter(n::name.eq(&chain_store.chain)))
            .set((
                n::head_block_hash.eq(&block_ptr.hash_hex()),
                n::head_block_number.eq(block_ptr.number as i64),
                n::head_updated.eq(diesel::dsl::now),
            ))
            .execute(&conn)
            .map(|x| x as u64);
        ret.map_err(Error::from)
    }
    async fn chain_update_balance_early_head(&self, early_block: &BlockPtr) -> Result<u64, Error> {
        use public::ethereum_balance as n;

        let chain_store = self.clone();
        let conn = self.pool.get().expect("can get a database connection");

        let ret = update(n::table.filter(n::name.eq(&chain_store.chain)))
            .set((
                n::early_head_block_hash.eq(&early_block.hash_hex()),
                n::early_head_block_number.eq(early_block.number as i64),
                n::early_head_updated.eq(diesel::dsl::now),
            ))
            .execute(&conn)
            .map(|x| x as u64);
        ret.map_err(Error::from)
    }

    async fn balance_address_list(&self, block_ptr: &BlockPtr) -> Result<Vec<Address>, Error> {
        let pool = self.pool.clone();
        let storage = self.storage.clone();

        let block_ptr_box = Box::new(block_ptr.clone());
        let ret = pool
            .with_conn(move |conn, _| {
                let block = block_ptr_box.deref();
                storage
                    .find_transaction_address(&conn, block)
                    .map_err(|e| StoreError::from(e).into())
            })
            .await
            .map_err(|e| Error::from(e));
        ret
    }
}

impl EthereumCallCache for ChainStore {
    fn get_call(
        &self,
        contract_address: ethabi::Address,
        encoded_call: &[u8],
        block: BlockPtr,
    ) -> Result<Option<Vec<u8>>, Error> {
        let id = contract_call_id(&contract_address, encoded_call, &block);
        let conn = &*self.get_conn()?;
        if let Some(call_output) = conn.transaction::<_, Error, _>(|| {
            if let Some((return_value, update_accessed_at)) =
                self.storage.get_call_and_access(conn, id.as_ref())?
            {
                if update_accessed_at {
                    self.storage
                        .update_accessed_at(conn, contract_address.as_ref())?;
                }
                Ok(Some(return_value))
            } else {
                Ok(None)
            }
        })? {
            Ok(Some(call_output))
        } else {
            Ok(None)
        }
    }

    fn set_call(
        &self,
        contract_address: ethabi::Address,
        encoded_call: &[u8],
        block: BlockPtr,
        return_value: &[u8],
        method_id: &[u8],
        call_args: Vec<String>,
    ) -> Result<(), Error> {
        let id = contract_call_id(&contract_address, encoded_call, &block);
        let conn = &*self.get_conn()?;
        conn.transaction(|| {
            self.storage.set_call(
                conn,
                id.as_ref(),
                contract_address.as_ref(),
                block.number as i32,
                return_value,
                method_id,
                call_args,
            )
        })
    }
}

/// The id is the hashed encoded_call + contract_address + block hash to uniquely identify the call.
/// 256 bits of output, and therefore 128 bits of security against collisions, are needed since this
/// could be targeted by a birthday attack.
fn contract_call_id(
    contract_address: &ethabi::Address,
    encoded_call: &[u8],
    block: &BlockPtr,
) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(encoded_call);
    hash.update(contract_address.as_ref());
    hash.update(block.hash_slice());
    *hash.finalize().as_bytes()
}

/// Support for tests
#[cfg(debug_assertions)]
pub mod test_support {
    use std::{str::FromStr, sync::Arc};

    use graph::prelude::{
        web3::types::H256, BlockNumber, BlockPtr, EthereumBlock, LightEthereumBlock,
    };

    // Hash indicating 'no parent'
    pub const NO_PARENT: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    /// The parts of an Ethereum block that are interesting for these tests:
    /// the block number, hash, and the hash of the parent block
    #[derive(Clone, Debug, PartialEq)]
    pub struct FakeBlock {
        pub number: BlockNumber,
        pub hash: String,
        pub parent_hash: String,
    }

    impl FakeBlock {
        pub fn make_child(&self, hash: &str) -> Self {
            FakeBlock {
                number: self.number + 1,
                hash: hash.to_owned(),
                parent_hash: self.hash.clone(),
            }
        }

        pub fn make_no_parent(number: BlockNumber, hash: &str) -> Self {
            FakeBlock {
                number,
                hash: hash.to_owned(),
                parent_hash: NO_PARENT.to_string(),
            }
        }

        pub fn block_hash(&self) -> H256 {
            H256::from_str(self.hash.as_str()).expect("invalid block hash")
        }

        pub fn block_ptr(&self) -> BlockPtr {
            BlockPtr::from((self.block_hash(), self.number))
        }

        pub fn as_ethereum_block(&self) -> EthereumBlock {
            let parent_hash =
                H256::from_str(self.parent_hash.as_str()).expect("invalid parent hash");

            let mut block = LightEthereumBlock::default();
            block.number = Some(self.number.into());
            block.parent_hash = parent_hash;
            block.hash = Some(self.block_hash());

            EthereumBlock {
                block: Arc::new(block),
                transaction_receipts: Vec::new(),
            }
        }
    }

    pub type Chain = Vec<&'static FakeBlock>;

    /// Store the given chain as the blocks for the `network` set the
    /// network's genesis block to `genesis_hash`, and head block to
    /// `null`
    pub trait SettableChainStore {
        fn set_chain(&self, genesis_hash: &str, chain: Chain);
    }
}

#[cfg(debug_assertions)]
impl test_support::SettableChainStore for ChainStore {
    fn set_chain(&self, genesis_hash: &str, chain: test_support::Chain) {
        let conn = self.pool.get().expect("can get a database connection");

        self.storage
            .set_chain(&conn, &self.chain, genesis_hash, chain);
    }
}
