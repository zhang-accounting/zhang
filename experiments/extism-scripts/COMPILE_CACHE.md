# 编译缓存接入与效果（2026-10-05）

当前分支已经把编译结果复用接入 `zhang-core` 的插件执行路径。
不复用活跃 WASM 实例；每次阶段执行和每次 Router 请求仍有独立的实例与宿主状态。

## 实现

- `RegisteredPlugin` 的克隆共享一个 `PluginRuntime`，由它保存模块字节及最近一次
  Manifest 对应的 `CompiledPlugin`。缓存命中时调用 `Plugin::new_from_compiled`。
- 注册、Processor、Mapper 和 Router 都使用该运行时。首次遇到一个配置会编译；
  之后配置相同的执行复用结果。注册阶段的旧配置行为保留，切换阶段配置时会重新编译。
- 比较完整 Manifest 设置，包括 config、网络/WASI 权限、超时与内存选项。
  模块字节属于单个注册快照，重载新模块时创建新运行时。缓存只保留最新配置，
  并发的首次编译在同一个锁内完成，避免同一配置被重复编译。
- 编译后的回调不捕获某次请求的账本、时间或文件权限；通过 Extism 实例 UUID 找到
  当前实例的宿主绑定。查找锁在执行宿主函数前释放。实例销毁时删除绑定，缓存仅
  持有上下文的弱引用，避免留下请求状态或账本引用。
- `PluginInstance::call` 使用原来的 Extism 调用方法，保留非零返回码、错误和超时处理。
  这次没有引入解释器实例池，也没有改变 Lua/Python 脚本加载方式。

## 配对计时

```sh
cargo run --locked --manifest-path experiments/extism-scripts/Cargo.toml --features lua-exceptions --bin performance -- --router-only
```

Apple M4、macOS 26.3，Rust 优化级别 2，沿用原有两个解释器 WASM。
每种语言使用同一份 10,000 笔交易账本、同一脚本、同一真实 BQL 查询及结果适配器。
旧生命周期基线每次构建插件，使用实验查询绑定；新实现直接调用生产
`RegisteredPlugin::execute_as_router`。两者均创建独立 WASM 实例、验证 HTTP 响应
和精确金额 `123400.00`。两次预热后交替执行并轮换先后顺序，每种情况记录 20 个样本。
Extism 原有磁盘编译缓存保持默认配置，基线没有人为禁用它。

| 语言 | 每次加载/编译，中位数 | 复用编译结果，中位数 | 本轮比值 | 新实现 p95 |
| --- | ---: | ---: | ---: | ---: |
| Python | 582.55 ms | 10.28 ms | 约 56.7 倍 | 28.87 ms |
| Lua | 35.72 ms | 8.89 ms | 约 4.0 倍 | 66.12 ms |

同轮 Rust 直接 BQL 的中位数为 Python 测试阶段 3.06 ms、Lua 阶段 5.85 ms。
机器上耗时有明显波动：Lua 缓存路径观察到 4.28–79.57 ms，Rust 直接查询在对应
阶段也观察到 2.33–50.07 ms。因此这些是本次本机观察值，不是固定延迟承诺，
也不是并发吞吐量或完整 HTTP 服务负载测试。

完整原始数值保存在 [COMPILE_CACHE.json](COMPILE_CACHE.json)。测试退出码为 0。
缓存降低了重复加载/编译成本；每个新实例仍需要初始化解释器。大 JSON 编解码与
脚本计算没有在这次改动中优化，原来的秒级数据往返问题仍需通过接口设计解决。

## 验证

- `cargo test --locked -p zhang-core --features plugin_runtime`：576 项全部通过。
  新检查覆盖编译复用、配置变化、宿主上下文释放、非零退出、WASM 全局变量隔离、
  重叠的 Router 查询各自使用自己的宿主，以及同路径模块修改后重载。
  原有文件权限、读取跟踪、内存边界和超时检查也通过。
- `cargo clippy --locked -p zhang-core --features plugin_runtime --all-targets -- -D warnings -D clippy::dbg_macro -A clippy::empty_docs`：通过。
- 根 workspace 与实验 package 的格式检查：通过。
- 编译缓存接入后的原 Python/Lua 功能验证：通过，真实 Processor 与 BQL Router
  仍正确执行，脚本错误恢复、超时和 JSON 往返也通过。

核心测试和 Clippy 的本机命令使用以下环境以复用实验构建目录和优化配置：

```sh
CARGO_TARGET_DIR=/Users/kilerd/Projects/zhang-script-runtimes/experiments/extism-scripts/target
CARGO_PROFILE_DEV_DEBUG=0
CARGO_PROFILE_DEV_OPT_LEVEL=2
CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=off
```

未修改生产 Cargo 依赖，未启用生产 Lua exceptions feature，未创建或合入 PR。
