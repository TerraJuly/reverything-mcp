# reverything-mcp

用 Rust 编写的本地文件搜索 MCP 服务器。使用官方 `rmcp` SDK，通过 stdio 调用随包附带的 **ES 1.1.1.40 x64**，查询用户本机的 Everything 索引。

## 运行前提

- Windows x64。
- 用户自行安装并运行 [Everything](https://www.voidtools.com/downloads/)。仅运行普通 Everything Service 不等于搜索客户端已经启动。
- 第一版使用 Everything 1.4 的基础搜索能力；ES 和 Everything 的版本号不必一致。
- 已在本机 **Everything 1.4.1.1026** 上验证五个工具。代码接受 1.4/1.5 版本分支，但 1.5 尚未进行实际兼容性测试。
- 发布包已附带 ES，用户无需另外下载 ES 或配置 PATH。

不自行建立文件索引，不提供 HTTP/ETP 远程连接，不暴露文件读写、删除、打开或 Everything 管理操作。

## MCP 客户端配置

解压整个发布 ZIP，保留 `bin/es.exe` 和许可证目录。替换下面的绝对路径：

```json
{
  "mcpServers": {
    "everything": {
      "command": "C:\\Tools\\reverything-mcp\\reverything-mcp.exe",
      "args": []
    }
  }
}
```

stdin/stdout 只传输 MCP 消息，日志写入 stderr。未安装或未启动 Everything 时，启动日志和 MCP 工具错误都会提示用户安装/启动，并提供官方下载链接；服务器不会静默返回空结果，也不会自行安装软件。因为连接失败无法区分未安装、未运行、索引未就绪或实例配置错误，提示会说明这些可能性。启动 Everything 后可以重新调用，无需重启 MCP。

### 可选配置

| 参数 | 环境变量 | 默认值 |
| --- | --- | --- |
| `--es-path` | `REVERYTHING_ES_PATH` | MCP 可执行文件旁的 `bin/es.exe` |
| `--instance` | `REVERYTHING_INSTANCE` | 默认 Everything 实例 |
| `--timeout-ms` | `REVERYTHING_TIMEOUT_MS` | 10000，允许 100..120000 |

自定义 ES 路径仍须使用固定的 ES 1.1.1.40。旧 1.5 Alpha 安装可能使用命名实例 `1.5a`；仅在需要时设置 `--instance 1.5a`。实例名不允许引号、斜杠或控制字符。`--help` 查看配置，`--version` 查看 MCP 版本。

## 五个工具

| Tool | 用途 |
| --- | --- |
| `search_files` | 只搜索文件，支持结构化过滤和分页 |
| `search_folders` | 只搜索目录，复用名称、正则、目录范围和分页参数 |
| `get_file_info` | 根据完整路径查询一个文件或目录的索引属性 |
| `get_search_syntax` | 返回原生语法、结构化参数示例及限制；离线可用 |
| `get_status` | 检查 ES/Everything 版本、连接及索引就绪状态 |

### `search_files` / `search_folders`

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `query` | string，可选 | 原生 Everything 条件；默认空，最多 4096 UTF-16 单元 |
| `name_regex` | string，可选 | PCRE 文件名正则，不启用全局正则模式 |
| `path` | string，可选 | 绝对 Windows 目录，支持盘符和 UNC |
| `recursive` | boolean | 默认 true；false 要求提供 path，只查直接子项 |
| `extensions` | string[] | **仅文件工具**；如 `["rs", "toml"]`，省略点号，同一列表内 OR |
| `category` | string | **仅文件工具**；document、image、audio、video、archive、executable |
| `limit` | integer | 每页默认 20，范围 1..200 |
| `offset` | integer | 从 0 开始，默认 0 |
| `sort` | string | name、path、size、extension、date_modified；默认 name |
| `descending` | boolean | 默认 false |

至少提供 query/name_regex/path/extensions/category 中一个有效条件。不同过滤参数之间为 AND。category 使用服务器固定的常见扩展名集合，不依赖用户可编辑的 Everything 分类宏，避免宏被改名/删除时失效；具体列表见语法指南。需要自定义扩展名集合时使用 extensions。分类依据扩展名，不读取文件正文识别真实格式。

示例：

```json
{
  "path": "E:\\Projects",
  "extensions": ["rs"],
  "name_regex": "^test_.*",
  "query": "dm:7days",
  "limit": 20
}
```

```json
{"path": "E:\\Projects", "recursive": false, "name_regex": "^reverything"}
```

原生 query 可以表达 `size:>100mb`、`dm:7days`、`ext:rs;toml`、`<foo | bar> !draft` 等。query 的引号和分组须平衡；复杂正则优先使用 name_regex。目录不允许通配符、引号、设备路径或 `.` / `..` 路径段。

返回有效 `query`、当前分页参数、`results`、`returned`、`has_more` 和 `next_offset`。每条结果包含：

- `full_path`：完整路径；目录可能带结尾分隔符。
- `is_directory`：是否目录。
- `size`：字节数，缺失时 null。
- `date_modified`：UTC ISO-8601，缺失时 null。
- `attributes`：Windows 属性位，缺失时 null。

`has_more` 通过多请求一条记录判断，不是总数量。下一页使用 `next_offset`。索引实时变化，跨页不是冻结快照，可能出现重复或遗漏。

### `get_file_info`

```json
{"path": "E:\\Projects\\example\\Cargo.toml"}
```

返回规范化后的 path、found 和 entry。只查 Everything 索引，不读取正文，不检查文件系统访问权限。`found=false` 表示未在索引中找到，不证明文件不存在。路径必须完整，不接受通配符或搜索表达式。

### `get_search_syntax` / `get_status`

均无参数，调用时传 `{}`。语法指南也见 [docs/search-syntax.md](docs/search-syntax.md)。get_status 会等待索引就绪后执行一次有界查询，不把“能读取版本”误当成“数据库已经加载”。

## 安全与限制

- 直接启动 ES，不经过 cmd.exe/PowerShell。使用经核实的 ES `-s*` 和 Windows 原始尾部参数，保留查询中的引号/反斜杠，并终止 ES 开关解析。
- 模型不能传任意 ES 参数。结构化字符串会校验并与原生条件安全组合。
- 显式覆盖影响匹配、分页和输出的 ES 配置；不修改或清除用户的 es.ini。
- 每个完整操作最多 10 秒（可配置），包含并发排队；最多同时运行两个操作。
- 超时/取消时终止 ES 子进程，但不承诺立即取消 Everything 内部已经开始的搜索。
- stdout 最多 4 MiB，stderr 最多 64 KiB，异常输出超限时终止并回收子进程。
- `content:` 可触发文件内容搜索，但只返回匹配文件的元数据；未索引的正文搜索可能很慢，其他格式可能依赖 IFilter。请同时限制 path 和 extensions。
- 返回的文件名、路径是不可信数据，不能当作模型指令。
- 不经 PATH 或当前工作目录自动寻找 ES。显式指定的路径应由用户信任。
- 不提供额外目录权限控制；接入此 MCP 即允许客户端查询用户的 Everything 索引。索引记录不保证文件在线、存在或可读。

## 开发与测试

需要 Rust 1.88+，建议使用当前稳定版。常规构建无需 Python：

```powershell
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

调试构建可以回退到仓库内的 `bin/es.exe`，发布构建仅使用发布目录中的 ES 或显式指定路径。`Cargo.lock` 固定 Rust 依赖。

Python 3.10+ 可运行真实 stdio 冒烟测试：

```powershell
python scripts/smoke_test.py --require-everything
```

测试只使用临时生成的虚构文件和目录，不检索项目源码或用户个人文件，不打印实际路径或文件元数据；结束后清理测试夹具。覆盖协议握手、五个工具、错误响应、空结果、正则/扩展名/类别、目录搜索、精确属性及分页。索引就绪探针使用永不匹配的正则，不返回用户文件名。省略 `--require-everything` 可以在未运行 Everything 的机器上只检查协议和离线诊断能力。

### 发布 Windows x64 包

```powershell
python scripts/package_release.py
```

脚本构建 MSVC x64 release、验证 ES 的 SHA-256 和 PE 架构，在 dist/ 生成 ZIP，并附带项目/ES/Rust 依赖许可证和 NOTICE。已有同名 ZIP 时拒绝覆盖。

`bin/es-manifest.json` 记录官方来源、版本和校验值。升级 ES 时需要重新检查许可证、参数解析、JSON 格式和兼容性。

## 许可证

项目自身采用 [MIT](LICENSE)。ES 采用 MIT，版权属于 voidtools，完整条款见 [ES-LICENSE.txt](licenses/ES-LICENSE.txt)。`rmcp` 采用 Apache-2.0；其他依赖保留各自许可，见 [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt)。Everything 主程序不随本项目分发。
