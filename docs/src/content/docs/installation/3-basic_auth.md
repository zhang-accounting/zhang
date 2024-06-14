---
title: Web Basic Authentication
description: This is a page in my Starlight-powered site
---

Zhang offers built-in basic authentication, which requires a plaintext username and password in the
format `{USERNAME}:{PASSWORD}` when launching the application. For example:

- `username:password`
- `admin:admin888`

You can enable basic authentication in two ways:

**Using the `auth` command-line parameter**:

```
docker run --name zhang kilerd/zhang:snapshot --auth admin:admin888
```

**Setting the `ZHANG_AUTH` environment variable**

```
docker run --name zhang -e "ZHANG_AUTH=admin:admin888" kilerd/zhang:snapshot
```

> Note that **command-line parameters** take precedence, so if both configuration options are provided, the command-line
> parameter will be used first.