
/**************************************************************
* CREATE TABLES
**************************************************************/

-- Stores balance
CREATE TABLE IF NOT EXISTS ethereum_balance (
    name VARCHAR PRIMARY KEY,
    namespace TEXT,
    head_block_hash VARCHAR,
    head_block_number BIGINT,
	net_version VARCHAR,
	genesis_block_hash VARCHAR,

    early_head_block_hash VARCHAR,
	early_head_block_number BIGINT,
    head_updated TIMESTAMP,
    early_head_updated TIMESTAMP
);