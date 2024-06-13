---
title: Load Beancount Data
description: This is a page in my Starlight-powered site
---

Zhang comes bundled with a Beancount processor, which automatically launches when the main file has a `.bc`, `.bean`,
or `.beancount` extension, starting the Zhang server.

To get started, simply specify the Beancount main file as the endpoint.

```shell
docker run --name zhang -v "/local/beancount:/data" -p "8000:8000" kilerd/zhang:snapshot --endpoint data.bean
```