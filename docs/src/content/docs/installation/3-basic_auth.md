---
title: Setting Up Basic Authentication
description: Learn how to secure your Zhang Accounting setup with basic authentication.
---

Zhang Accounting supports basic authentication to help secure your data. This method requires a username and password in the format `{USERNAME}:{PASSWORD}` when accessing the application. For instance:

- `username:password`
- `admin:admin888`

There are two ways to enable basic authentication:

**Using the `auth` command-line parameter**:

```
docker run --name zhang kilerd/zhang:snapshot --auth admin:admin888
```

**Setting the `ZHANG_AUTH` environment variable**

```
docker run --name zhang -e "ZHANG_AUTH=admin:admin888" kilerd/zhang:snapshot
```

> Note: **Command-line parameters** have priority over environment variables. If both are provided, the command-line parameter will be used.

### Troubleshooting Common Authentication Issues

When setting up basic authentication, you might encounter some common issues. Here are a few troubleshooting tips:

- **Incorrect Credentials**: Ensure that the username and password are correctly entered. Remember that the format is `{USERNAME}:{PASSWORD}`.
- **Environment Variables Not Recognized**: If using Docker, ensure that the environment variable is correctly passed to the container. Double-check the syntax used in the `docker run` command.
- **Authentication Prompt Not Appearing**: If the authentication prompt does not appear when accessing Zhang Accounting, ensure that basic authentication is correctly enabled through either the command-line parameter or the environment variable.

By following these steps and troubleshooting tips, you can effectively set up and manage basic authentication for Zhang Accounting, adding an extra layer of security to your accounting data.
