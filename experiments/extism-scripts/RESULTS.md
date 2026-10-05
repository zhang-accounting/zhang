# 验证结果（2026-10-05）

验证基线：main `d3dc084f`；Extism `1.30.0`；Python PDK `0.1.5`；
lua.wasm `0.2.0`；WASI SDK `34`；dkjson `2.11`。
构建和测试均在 macOS arm64 上执行。生产 workspace 没有修改。

两个运行时均构建成功，并通过以下实际执行检查：

| 检查 | Python | Lua |
| --- | --- | --- |
| 同一 WASM 实例接收不同源码，返回 3 和 30，无需重编译运行时 | 通过 | 通过 |
| 调用真实 `zhang_now` 和 `zhang_emit_error` | 通过 | 通过 |
| 脚本抛错后，同一实例再次成功执行源码 | 通过 | 通过 |
| 无限循环由 Extism 的 100 ms 配置超时停止 | 通过，观察到 180 ms | 通过，观察到 104 ms |
| 空对象、空数组、null、false、超过 2^53 的整数、精确金额字符串往返 | 通过 | 通过 |
| 真实 Processor 经 `zhang_read_file` 读取独立脚本 | 通过 | 通过 |
| 脚本文件加入账本 `extra_inputs` | 通过 | 通过 |
| 真实 Router 经 `zhang_query` 执行账本 BQL | 通过 | 通过 |
| `12.34 + 0.01` 的费用报表结果为精确的 `12.35` | 通过 | 通过 |

这些结果来自 `cargo run --locked --manifest-path experiments/extism-scripts/Cargo.toml --features lua-exceptions`，
退出码为 0。原始结果保存在本地 `artifacts/results.json`。

| 产物 | 字节数 | MiB |
| --- | ---: | ---: |
| `artifacts/python-runtime.wasm` | 10,914,613 | 10.41 |
| `artifacts/lua-runtime.wasm` | 826,721 | 0.79 |

本次实例创建观察值为 Python 5.23 秒、Lua 0.18 秒；源码调用分别在约
0.34–6.88 ms 和 1.48–2.74 ms 范围。首次编译、运行时缓存和机器上的并发任务
会影响这些数字；它们只是本机烟雾测试记录，不是性能比较基准。

## 实际发现的接入条件

- 默认 Extism 配置下，Python 全部通过，Lua 加载被拒绝：
  `exceptions proposal not enabled`。此 Lua 构建需要 `extism/wasmtime-exceptions`。
  该 feature 只在实验 package 启用，生产依赖没有变更。
- 初版 Lua JSON 库把空对象编码成空数组，真实 Processor 因此被 Zhang 拒绝。
  换成保留对象/数组标记和显式 null 的 dkjson 后，真实账本与独立 JSON 往返检查通过。
- Router 阶段的文件接口仍遵循现有权限规则，禁止读取脚本文件，因此实验通过
  config 传入 Router 源码。正式脚本加载器还需要负责加载、跟踪并传递源码。

正式集成还需要脚本加载器、每个脚本的插件身份和依赖方案。当前例子没有实现
`plugin "business.py"` / `plugin "business.lua"`，没有验证第三方 Python 包，
也没有更改现有报表业务或前端。
