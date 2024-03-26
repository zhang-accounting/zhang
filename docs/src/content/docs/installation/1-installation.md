---
title: How to install
description: This is a page in my Starlight-powered site
---

Currently we recommend to use docker to deploy zhang service, and we can pull the docker image via:

```shell
docker pull kilerd/zhang:0.1
```

To make zhang working, we need to configurate some variables:

- root of data：let zhang know where we store the data
- endpoint of main file：we use `main.zhang` as default endpoint, and we can change it by parameter `--endpoint`
- datasource：zhang support multiple datasource, and the default value is local file system `fs`, and we can change it by
  param `--source`

## Difference of versions

zhang provide some different version of docker images, here are differences:

- `kilerd/zhang:snapshot` the version linked to the latest of codebase, including untested features
- `kilerd/zhang:latest` latest stable version
- `kilerd/zhang:0.1` latest stable version

Go to [Docker Hub](https://hub.docker.com/r/kilerd/zhang/tags) to check the detail of each version

## Deploy via Docker

you can start your zhang server via:

```shell
docker run --name zhang -v "/local/zhang/path:/data" -p "8000:8000" kilerd/zhang:0.1
```

