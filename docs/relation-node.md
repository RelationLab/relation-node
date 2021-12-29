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
    -e SUBGRAPH_ALLOWEDLIST='0x73a721d73d7469c583303be2c9DE41232DbD20C3,0x74467c63f3200A8a876E385d3e492aeB4b0D024B' \
    graph-node
```