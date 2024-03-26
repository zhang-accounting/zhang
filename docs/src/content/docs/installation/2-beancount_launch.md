---
title: 加载 beancount 数据
description: This is a page in my Starlight-powered site
---


zhang 自带了 beancount 的处理器，也会在**主文件** `endpoint` 的文件后缀为 `.bc` 或 `.bean` 时，以 **beancount 处理器**启动
zhang 服务端。

因此你需要在启动时指定 endpoint 为 beancount 主文件。

```shell
docker run --name zhang -v "/local/beancount:/data" -p "8000:8000" kilerd/zhang:snapshot --endpoint data.bean
```