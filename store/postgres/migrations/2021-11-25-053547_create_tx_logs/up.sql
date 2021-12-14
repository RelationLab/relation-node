/**************************************************************
* CREATE TABLES
**************************************************************/

-- Stores transactions from each Ethereum network
CREATE TABLE IF NOT EXISTS ethereum_transactions (
    hash VARCHAR PRIMARY KEY NOT NULL,
    block_hash VARCHAR NOT NULL,
    block_number BIGINT NOT NULL,
    "from" VARCHAR NOT NULL,
    gas VARCHAR NOT NULL,
    gas_price VARCHAR NOT NULL,
    max_fee_per_gas VARCHAR,
    max_priority_fe_per_gas VARCHAR,
    input TEXT NOT NULL,
    nonce VARCHAR NOT NULL,
    transaction_index VARCHAR NOT NULL,
    value VARCHAR NOT NULL
);

-- Stores receipts from each Ethereum network
CREATE TABLE IF NOT EXISTS ethereum_receipts (
    id VARCHAR NOT NULL,
    block_hash VARCHAR NOT NULL,
    block_number VARCHAR NOT NULL,
    data TEXT,
    topics VARCHAR UNIQUE NOT NULL,
    address VARCHAR NOT NULL,
    removed BOOLEAN NOT NULL,
    log_index BIGINT NOT NULL,
    log_type VARCHAR,
    transaction_hash VARCHAR NOT NULL,
    transaction_index BIGINT NOT NULL,
    transaction_log_index VARCHAR
);






/**************************************************************
* ADD etherum_networks COLUMNS
**************************************************************/

ALTER TABLE ethereum_networks
	ADD COLUMN early_head_block_hash VARCHAR,
	ADD COLUMN early_head_block_number BIGINT,
    ADD COLUMN early_head_updated TIMESTAMP,
    ADD COLUMN head_updated TIMESTAMP
