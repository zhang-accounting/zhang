---
title: how to upgrade
---

Here is the guide to updating Zhang to the latest version.

## Docker user

If you are using Zhang within Docker, you can use the following command to pull the latest Docker image and then restart
your Docker container:

```shell
docker pull Kilerd/zhang:latest
````

## CLI user

If you are a CLI user, you can directly use the update command. Zhang will detect your operating system architecture and
replace your local Zhang executable file with the latest version:

```shell
zhang update
```