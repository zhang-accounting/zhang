# 如何安装


目前zhang只推荐使用 docker 的方式进行部署，你可以通过一下的命令拉取镜像
```shell
docker pull kilerd/zhang:snapshot
```

!> 目前 zhang 尚未发布稳定版，提供的 `snapshot` 版本为main 分支的最新开发版。

为了让 zhang 正常运行，你需要提供几个配置：
- 数据根目录：用来定义文档和数据的存放相对路径
- 主文件：默认是数据个目录下的`main.zhang`, 也可以通过 `--endpoint` 参数来替换
- 数据源：zhang支持多种数据源，默认是本地文件系统`fs`，可以通过 `--source` 来替换成其他支持的数据源。 切换到不同数据源时，需要提供对应的配置项以完成数据源的连接。


### docker 部署
你可以通过以下的命令启动 zhang
```shell
docker run --name zhang -v "/local/zhang/path:/data" -p "8000:8000" kilerd/zhang:snapshot
```

