---
title: Installation Guide
description: Comprehensive steps to install and verify Zhang Accounting.
---

Zhang Accounting is designed to be flexible and accessible, offering various deployment options to suit different environments. This guide focuses on deploying Zhang using Docker, which simplifies the installation process and ensures consistency across different setups.

## Prerequisites

Before proceeding with the installation, ensure you have Docker installed on your system. Docker provides a containerized environment that encapsulates Zhang Accounting, making it easy to deploy and manage without worrying about dependencies.

## Pulling the Docker Image

The first step is to pull the Zhang Accounting Docker image from Docker Hub:

```shell
docker pull kilerd/zhang:latest
```

## Configuration Overview

To get Zhang Accounting up and running, you'll need to configure a few key parameters:

- **Data Root**: Specifies where Zhang should store its data.
- **Endpoint of Main File**: By default, Zhang uses `main.zhang` as the main file. This can be customized using the `--endpoint` parameter.
- **Data Source**: Zhang supports multiple data sources. The default data source is the local file system (`fs`), which can be changed using the `--source` parameter.

## Docker Deployment

Deploying Zhang Accounting with Docker is straightforward. Here's a command that starts the Zhang server, mapping a local directory to the container for data storage:

```shell
docker run --name zhang -v "/local/zhang/path:/data" -p "8000:8000" kilerd/zhang:latest
```

## Verifying the Installation

After starting the Zhang server, it's important to verify that the installation was successful. You can do this by accessing the Zhang web interface through your browser. By default, Zhang listens on port 8000, so you can navigate to `http://localhost:8000` to check if the server is running correctly.

If you encounter any issues during installation or while verifying the server, consult the troubleshooting section or reach out to the Zhang community for support.

## Version Differences

Zhang Accounting offers different versions of its Docker images to cater to various needs:

- `kilerd/zhang:snapshot`: Linked to the latest codebase, including untested features.
- `kilerd/zhang:latest`: The latest stable version.
- `kilerd/zhang:0.1`: A specific stable version.

For more details on each version, visit [Docker Hub](https://hub.docker.com/r/kilerd/zhang/tags).

By following these steps, you should have a running instance of Zhang Accounting. For further configuration options and advanced usage, refer to the subsequent sections of the documentation.
