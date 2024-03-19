# 如何安装

目前zhang只推荐使用 docker 的方式进行部署，你可以通过一下的命令拉取镜像

```shell
docker pull kilerd/zhang:0.1
```

为了让 zhang 正常运行，你需要提供几个配置：

- 数据根目录：用来定义文档和数据的存放相对路径
- 主文件：默认是数据个目录下的`main.zhang`, 也可以通过 `--endpoint` 参数来替换
- 数据源：zhang支持多种数据源，默认是本地文件系统`fs`，可以通过 `--source` 来替换成其他支持的数据源。
  切换到不同数据源时，需要提供对应的配置项以完成数据源的连接。

## 版本选择

目前 zhang 提供了两种不同的版本 docker 构建：

- `kilerd/zhang:snapshot` 这是关联到代码仓库最新更改的版本，可能包括了未经测试的功能，同时也会带来一定的不稳定性
- `kilerd/zhang:latest` 最新的稳定版本
- `kilerd/zhang:0.1` 稳定的版本

具体的版本信息可以前往 [Docker Hub](https://hub.docker.com/r/kilerd/zhang/tags) 查看

### docker 部署

你可以通过以下的命令启动 zhang

```shell
docker run --name zhang -v "/local/zhang/path:/data" -p "8000:8000" kilerd/zhang:0，1
```

