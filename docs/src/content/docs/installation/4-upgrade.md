---
title: Upgrading Zhang Accounting
description: Step-by-step guide to safely upgrade Zhang Accounting to the latest version.
---

Upgrading Zhang Accounting ensures you have the latest features, improvements, and security updates. This guide provides detailed steps and precautions for a smooth upgrade process.

## Docker Users

For those running Zhang Accounting within Docker, follow these steps to upgrade:

1. **Pull the Latest Docker Image**: Fetch the most recent version of the Zhang Accounting Docker image using the command below:

   ```shell
   docker pull kilerd/zhang:latest
   ```

2. **Stop the Current Container**: If your Zhang container is running, stop it using:

   ```shell
   docker stop <your-container-name>
   ```

3. **Remove the Old Container**: To avoid conflicts, remove the existing container:

   ```shell
   docker rm <your-container-name>
   ```

4. **Start a New Container**: Launch a new container with the updated image:

   ```shell
   docker run --name <your-container-name> -d -p 8000:8000 kilerd/zhang:latest
   ```

   Replace `<your-container-name>` with your actual container name.

## CLI Users

If you installed Zhang Accounting using the CLI, you can upgrade directly through the command line:

1. **Run the Update Command**: Execute the following command to update Zhang. The system will automatically detect your operating system and architecture, replacing the local executable with the latest version:

   ```shell
   zhang update
   ```

## Verifying the Upgrade

After upgrading, it's important to verify that the new version is running correctly:

1. **Check Version**: Use the `zhang --version` command to confirm the installed version matches the latest release.
2. **Test Functionality**: Perform basic operations to ensure Zhang Accounting is functioning as expected.

## Troubleshooting

If you encounter issues during the upgrade process, consider the following:

- **Check Compatibility**: Ensure your data files are compatible with the new version. Review the release notes for any breaking changes.
- **Consult Documentation**: The latest documentation may have specific instructions or notes regarding the upgrade.
- **Seek Support**: If problems persist, seek assistance from the Zhang Accounting community or support channels.

Upgrading regularly is crucial for accessing new features and maintaining the security and stability of your accounting system. Follow these steps to keep your Zhang Accounting installation up-to-date.
