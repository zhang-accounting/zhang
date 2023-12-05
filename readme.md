<img align="right" width="256" height="256" src="/assets/logo.jpg">

# 账 Zhang

a plain text double-accounting tool.

![GitHub Workflow Status (with event)](https://img.shields.io/github/actions/workflow/status/zhang-accounting/zhang/build-latest.yml)
[![](https://codecov.io/gh/zhang-accounting/zhang/branch/main/graph/badge.svg?token=AVM0HNGF91)](https://codecov.io/gh/zhang-accounting/zhang)
![Crates.io (recent)](https://img.shields.io/crates/dr/zhang)
![Docker Pulls](https://img.shields.io/docker/pulls/kilerd/zhang)
[![](https://img.shields.io/docsrs/zhang)](docs.rs/zhang)
![](https://img.shields.io/crates/l/zhang)

- Online Demo: [Online Demo](https://zhang-accounting.zeabur.app/)
- Documentation: [Documentation](zhang-accounting.github.io/zhang/)

## Features
 - **Independent Direcitve**: all directives in zhang are independent, you can write them in any file with any order.
 - **More Precise Control**: features, like commodity decimal precision and datetime supported for directive, provide more control
 - **Document Management Enhancement**: zhang has a good document management feature to allow you collect and control document easiler and effective, like receipts.

### Compatibility with beancount
beancount and zhang are both text based accounting tools, and they are some familiar.

But zhang deprecates some directives, like `note`, `pad` and `push_tag`, and aslo evolves some direcitves, like `balance`. So your beancount file may not be compatible with zhang, we will provide a tool to convert beancount format to zhang format, and vice versa.


## Installation
### With Docker
The image is available at `kilerd/zhang`, you shold persist the `/data` folder, which contains all your zhang accounting files.

**NOTE: currently zhang only provide the snapshot version.**

Example for docker compose:
```yaml
version: '3'

volumes:
  zhang_data:
    driver: local

services:
  zhang:
    image: kilerd/zhang:snapshot
    ports:
      # For the web front-end, you may change the port
      - "8000:8000"
    volumes:
      - "zhang_data:/data"
      # Alternatively, you can mount a local folder
      # - "./zhang_data:/data"
```
#### Compatibility with beancount
zhang has its own file structure, which start with endpoint `main.zhang`. if you wanna start zhang with your beancount file, you may need to specify the endpoint of main file. 

if you are using cli command:
```shell
zhang server /data --endpoint main.bean
```
if you are using docker run command:
```shell
docker run --name zhang -v "/local_beancount_data:/data" -p "18000:8000" kilerd/zhang:snapshot --endpoint main.bc
```
or with docker-compose file:
```yaml
services:
  zhang:
    image: kilerd/zhang:snapshot
    command: --endpoint main.bean
```

### From source
to compile the project, you'll need:
- node 16: used for frontend react project
- rust

then you need to build the frontend project first:
```shell
$ cd frontend
$ yarn & yarn build
```

then compile the backend service:
```shell
$ cd ..
$ cargo build --release
```

## Development

### Flamegraph
command is `CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph --root  -- parse ./example-accounting`
