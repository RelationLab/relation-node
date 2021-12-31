#### build docker image

```
    ./docker/build.sh
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
    -e SUBGRAPH_ALLOWLIST_FILEPATH='./allowlist.json' \
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