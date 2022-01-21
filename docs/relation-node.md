
### Running Local Graph Node
1. Install [IPFS](https://docs.ipfs.io/install/) and run
2. Install [PostgreSQL](https://www.postgresql.org/download/) and ```createdb graphtest.```
3. build docker image
```
    ./docker/build.sh
```
* Command-Line Interface
```
USAGE:
    graph-node [FLAGS] [OPTIONS] --ethereum-ipc <NETWORK_NAME:FILE> --ethereum-rpc <NETWORK_NAME:URL> --ethereum-ws <NETWORK_NAME:URL> --ipfs <HOST:PORT> --postgres-url <URL>

FLAGS:
        --debug      Enable debug logging
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --admin-port <PORT>                           Port for the JSON-RPC admin server [default: 8020]
        --elasticsearch-password <PASSWORD>
            Password to use for Elasticsearch logging [env: ELASTICSEARCH_PASSWORD]

        --elasticsearch-url <URL>
            Elasticsearch service to write subgraph logs to [env: ELASTICSEARCH_URL=]

        --elasticsearch-user <USER>                   User to use for Elasticsearch logging [env: ELASTICSEARCH_USER=]
        --ethereum-ipc <NETWORK_NAME:[CAPABILITIES]:FILE>
            Ethereum network name (e.g. 'mainnet'), optional comma-seperated capabilities (eg full,archive), and an Ethereum IPC pipe, separated by a ':'

        --ethereum-polling-interval <MILLISECONDS>
            How often to poll the Ethereum node for new blocks [env: ETHEREUM_POLLING_INTERVAL=]  [default: 500]

        --ethereum-rpc <NETWORK_NAME:[CAPABILITIES]:URL>
            Ethereum network name (e.g. 'mainnet'), optional comma-seperated capabilities (eg 'full,archive'), and an Ethereum RPC URL, separated by a ':'

        --ethereum-ws <NETWORK_NAME:[CAPABILITIES]:URL>
            Ethereum network name (e.g. 'mainnet'), optional comma-seperated capabilities (eg `full,archive), and an Ethereum WebSocket URL, separated by a ':'

        --http-port <PORT>                            Port for the GraphQL HTTP server [default: 8000]
        --ipfs <HOST:PORT>                            HTTP address of an IPFS node
        --node-id <NODE_ID>                           a unique identifier for this node [default: default]
        --postgres-url <URL>                          Location of the Postgres database used for storing entities
        --subgraph <[NAME:]IPFS_HASH>                 name and IPFS hash of the subgraph manifest
        --ws-port <PORT>                              Port for the GraphQL WebSocket server [default: 8001]
```

#### example usage

```
    docker run -it \
    -p 8000:8000 \
    -p 8001:8001 \
    -p 8020:8020 \
    -p 8030:8030 \
    -e postgres_host=10.0.191.31 \
    -e postgres_port=5432 \
    -e postgres_user=graphtest \
    -e postgres_pass=graphtest \
    -e postgres_db=graphtest \
    -e ipfs=10.0.191.31:5001 \
    -e ethereum=privnet:http://10.0.191.31:8545 \
    -e SUBGRAPH_ALLOWLIST_FILEPATH='/allowlist.json' \
    graph-node
```

* SUBGRAPH_ALLOWLIST_FILEPATH (allowlist.json)

```
{
    "allowlist" : [
        "0xd26114cd6ee289accf82350c8d8487fedb8a0c07",
        "0x209c4784ab1e8183cf58ca33cb740efbf3fc18ef",
        "0xdac17f958d2ee523a2206206994597c13d831ec7",
        "0x36928500bc1dcd7af6a2b4008875cc336b927d57",
        "0xe0bdaafd0aab238c55d68ad54e616305d4a21772",
        "0x798d1be841a82a273720ce31c822c61a67a601c3",
        "0xd13c7342e1ef687c5ad21b27c2b65d772cab5c8c",
    ]
}
```