---
title: Launching with Beancount Data
description: Detailed instructions on initiating Zhang Accounting with Beancount data files.
---

Zhang Accounting seamlessly integrates with Beancount, allowing users to launch the application using Beancount data files. This functionality is particularly useful for those transitioning from Beancount or looking to utilize Beancount's plain text accounting format. This guide will walk you through the process of launching Zhang Accounting with Beancount data, including common configurations and potential issues you might encounter.

## Beancount Processor Integration

Zhang is equipped with a Beancount processor that automatically activates when the main file has a `.bc`, `.bean`, or `.beancount` extension. This processor facilitates the use of Beancount files, ensuring a smooth transition and operation within the Zhang Accounting ecosystem.

### Launching Zhang with Beancount Data

To launch Zhang Accounting with your Beancount data, you need to specify the Beancount main file as the endpoint. Here's how you can do it using Docker:

```shell
docker run --name zhang -v "/path/to/your/beancount/files:/data" -p "8000:8000" kilerd/zhang:latest --endpoint main.bean
```

In this command:
- Replace `"/path/to/your/beancount/files"` with the actual path to your Beancount files.
- `main.bean` is the name of your Beancount main file. Adjust this according to your file's name.

## Common Configurations

When launching Zhang with Beancount data, you might want to customize certain aspects of its operation. Here are some common configurations you might consider:
- **Custom Port**: If you wish to use a different port than the default `8000`, you can modify the `-p` parameter accordingly.
- **Volume Mapping**: Ensure the volume mapping `-v` parameter correctly points to your Beancount files' location for Zhang to access them.

## Troubleshooting

Encountering issues while launching Zhang with Beancount data is not uncommon. Here are a few tips to help you troubleshoot:
- **File Permissions**: Ensure Zhang has the necessary permissions to access your Beancount files.
- **File Path**: Double-check the file path and main file name specified in the Docker command. A typo or incorrect path can prevent Zhang from accessing your data.
- **Docker Issues**: If you're facing issues related to Docker, such as container not starting, consult Docker's documentation or forums for assistance.

Launching Zhang Accounting with Beancount data combines the simplicity and power of plain text accounting with Zhang's advanced features. By following the steps outlined in this guide, you can get started with Zhang using your existing Beancount files, making your accounting process more efficient and streamlined.
