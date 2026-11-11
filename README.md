# Nocturne

中文 | [English](README_EN.md)

**Nocturne** 是一个本地优先、支持版本历史与可逆写入的记忆管理引擎，以单个自包含可执行文件（`nocturne`）交付。SQLite 与全文检索引擎已内置，运行时无需外部运行时或后台服务。

本项目受 [Dataojitori/nocturne_memory](https://github.com/Dataojitori/nocturne_memory) 启发，在 fork 该仓库后进行了重写和修改。

版本统一在 [`Cargo.toml`](Cargo.toml) 中管理：程序版本使用 `workspace.package.version`，可通过 `nocturne --version` 查看；数据库格式标记使用 `package.metadata.nocturne.schema-version`，不随程序发版自动递增。数据库须通过 `nocturne init` 在新路径显式创建；程序直接拒绝非本格式或旧格式的数据库文件，不提供旧格式升级。除基础读写与全文检索外，引擎内置批量目录导入（`import-dir`）、可读目录导出（`export-dir`）、历史版本恢复（`restore`）、变更集回滚（`review rollback`）和在线一致性备份（`backup`）。

---

## 构建与安装

项目使用 **Rust 2024 edition**，请安装当前 stable Rust 工具链。在仓库根目录执行：

```sh
cargo build --locked --release
cargo install --locked --path .
nocturne --help
```

- `Cargo.lock` 纳入版本控制，构建与安装建议使用 `--locked`。
- CI 在 Ubuntu、macOS、Windows 上执行格式检查、Clippy、release 构建和单文件验收，并将二进制作为工作流产物（Workflow Artifacts）上传，当前不发布 GitHub Release。
- CI 构建的 Windows release 产物启用了静态 CRT（`+crt-static`），无需额外安装 Visual C++ 运行库。在本地 Windows MSVC 环境如需构建静态 CRT 可执行文件，请显式指定：
  ```sh
  RUSTFLAGS='-C target-feature=+crt-static' cargo build --locked --release --bin nocturne
  ```

---

## 供 Agent 使用的 Skills

[`skills/`](skills/README.md) 提供两个可独立安装的通用技能：`nocturne` 用于日常检索、记录和纠错；`nocturne-audit` 用于证据与检索路径审计，默认只读。两者不依赖特定操作系统或 agent，也不需要 MCP。

将这些文件提交并推送到仓库后，可通过 [Vercel Skills CLI](https://github.com/vercel-labs/skills) 安装：

```text
npx skills add https://github.com/lll9p/Nocturne --skill nocturne nocturne-audit
```

未发布的本地版本可在仓库根目录使用 `npx skills add . --skill nocturne nocturne-audit`。安装器会让你选择 agent；加 `--global` 为用户级安装。**安装 skill 不会安装 Nocturne 二进制**，仍需单独安装 CLI，并给 agent 提供获准操作的数据库和 namespace。完整说明见 [Skills 指南](skills/README.md)。

---

## 快速开始

以下命令可按顺序直接执行，完整走通初始化数据库、写入记忆、检索、局部修补与批次审阅闭环：

```sh
# 1. 初始化新数据库（仅 init 会创建数据库文件，且拒绝覆盖已有文件）
nocturne --db memory.db init

# 2. 创建一条记忆：在域根 core:// 下创建子节点 core://architecture，并归入 setup 审阅批次
printf '系统采用单二进制 SQLite 存储，支持全文检索与可逆变更。\n' > note.md
nocturne --db memory.db --batch setup create core:// --title architecture --file note.md --priority 1 --disclosure '核心架构说明'

# 3. 读取记忆（含正文、版本 ID、边元数据、子节点与节点属性）
nocturne --db memory.db read core://architecture

# 4. 全文检索与局部精确修补
nocturne --db memory.db search '全文检索'
nocturne --db memory.db --batch setup patch core://architecture --old '全文检索' --new 'jieba 分词全文检索'

# 5. 查看待审阅记录、对比 setup 批次的净差异，并审阅接受
nocturne --db memory.db review list --status pending
nocturne --db memory.db review diff setup --by-batch
nocturne --db memory.db review accept --all
```

各子命令的完整选项可通过 `nocturne <COMMAND> --help` 查看。

---

## 核心概念与数据模型

### 1. 数据库路径与命名空间
- **显式路径与显式初始化**：除无需数据库的 `diff` 外，所有命令均须通过 `--db <PATH>` 或环境变量 `NOCTURNE_DB` 指定数据库路径（无隐式系统目录兜底）；除 `init` 外，任何命令都不会因路径拼写错误而自动创建数据库。
- **定位顺序**：命令行参数 `--db <PATH>` 与 `--namespace <NAME>` 优先级高于环境变量 `NOCTURNE_DB` 与 `NOCTURNE_NAMESPACE`；未指定 `--namespace` 时默认为空字符串 `""`。在自动化脚本中建议始终显式传入 `--db` 和 `--namespace`。
- **全局参数位置**：`--db`、`--namespace`、`--json` 与写入归组标签 `--batch <BATCH_ID>` 均为全局参数，放在子命令之前或之后均可生效。

### 2. 节点（Node）、路径（URI）与边属性（Edge Metadata）
- **URI 与别名**：每条 URI（如 `core://architecture`）表示当前命名空间内的一条路径。域名（`domain`）与新建路径段均支持 Unicode 字母、数字、组合标记、`-` 和 `_`（不限于 ASCII，例如 `文献://水泥/阿利特`）。多条 URI（别名）可以指向同一个稳定的节点 UUID，共享同一份正文与节点 JSON 属性。
- **边属性与节点属性分离**：
  - **边属性（Edge Metadata）**：优先级 `priority`（整数 `0–999`，数值越小优先级越高）和披露说明 `disclosure` 绑定在图的**边**上，用来控制树形关系、检索排序和复习权重。显式创建的每条 `alias <SOURCE> <DESTINATION>` 拥有独立的边元数据，修改其 `priority`/`disclosure` 不会覆盖原路径（仅在对含子节点的父路径建立别名并自动派生后代路径时，派生子路径共享对应的子边）。
  - **节点属性（Node Attributes）**：通过 `attrs` 管理的 JSON 对象绑定在**节点**上，由指向该节点的所有别名共享。

### 3. 全库版本 ID（Database-wide Version ID）
- 每次创建或修改记忆正文时，系统会生成一条新的内容版本记录，分配全库递增的整数 `memory_id`（在输出和 `--expect-version` 中显示为 `version`）。
- **注意**：`version` 是**全库范围的唯一整数 ID**，不是单个节点内部从 1 开始递增的局部计数器。执行 `restore` 或传入 `--expect-version` 时，请使用 `read` 或 `history` 返回的实际版本号。

### 4. 写入即生效与三种移除/撤销操作
- **写入立即生效**：每次写入都在同一 SQLite 事务中同时更新业务数据与可逆审计记录（changeset，其 `change_id` 为 UUID）。`review accept` 只是将**已经生效**的变更标记为“已审阅”，并非发布待生效草稿。
- **区分三种移除/撤销操作**：
  1. `unlink <URI>`：仅移除当前命名空间的一条路径，并保护依赖子路径不会因此变成不可达。若节点仍有其他别名则继续存在；若在所有命名空间都已无路径，则成为可恢复的无路径节点（`orphans`）。
  2. `review rollback <ID>`：根据审计快照逆向回滚某次变更或批次。
  3. `purge <MEMORY_ID>`：对已废弃的历史版本（`deprecated`）执行不可逆的物理清理。

---

## 日常使用指南

### 写入、编辑与节点属性

```sh
# 创建子节点并附带初始节点属性 JSON
nocturne --db memory.db create core:// --title 项目规范 --file spec.md --priority 1 --attrs-file attrs.json

# 更新正文与边元数据（同一事务、同一 changeset 提交；省略 --file 时仅更新元数据且不生成新版本）
nocturne --db memory.db update core://项目规范 --file spec_v2.md --priority 2 --disclosure '开发前必读'

# 唯一字面文本块修补（支持 --old/--new 或 --old-file/--new-file，无通配/模糊/转义改写，前置校验 --expect-version）
nocturne --db memory.db patch core://项目规范 --old-file old_part.md --new-file new_part.md

# 乐观并发控制：传入 read 返回的当前实际版本号 <VERSION>（update/patch/append/meta/attrs/alias/move/rename/unlink/trigger 均支持）
nocturne --db memory.db update core://项目规范 --file spec_v3.md --expect-version <VERSION>

# 追加正文与单独修改边元数据（将 disclosure 置为空字符串 '' 可清空显示文本；均返回更新后的完整节点）
nocturne --db memory.db append core://项目规范 --file extra.md
nocturne --db memory.db meta core://项目规范 --priority 1 --disclosure ''

# 查看与编辑共享节点属性（--set 值始终按顶层字符串存储，不推断数字/布尔/嵌套结构）
nocturne --db memory.db attrs core://项目规范
nocturne --db memory.db attrs core://项目规范 --set status=active --set owner=team --unset draft
nocturne --db memory.db attrs core://项目规范 --file attrs.json

# 管理触发关键词（add/remove/edit 均为幂等操作并返回统一结果；纯 CJK 词同步更新命名空间分词词典）
nocturne --db memory.db trigger add core://项目规范 架构 规范
nocturne --db memory.db trigger edit core://项目规范 --add 指南 --remove 规范
nocturne --db memory.db trigger list core://项目规范
```

- **路径段语义**：`create <PARENT_URI> --title <NAME>` 将 `NAME` 作为子路径段追加到已有父路径下，并非自由格式的显示标题。
- **字节级保真与幂等写入**：正文按原始 UTF-8 字节存储（保留 NUL、CRLF、组合字符及末尾换行状态，不对新写入做 NFC 规范化）。若写入后的正文与边元数据均无净变化，返回 `changed: false`，不会产生新版本、changeset 或空批次。
- **属性命名提示**：若节点属性 JSON 中包含名为 `priority` 或 `disclosure` 的自定义字段，它们不会修改边元数据，而会在成功结果或预览根层的 `warnings` 数组中给出提示。
- **标准输入（stdin）**：`create`、`update`、`append`、`attrs`、`preset create/update` 支持 `--file -` 从 stdin 读取；`create` 与 `import-dir` 支持 `--attrs-file -`；`patch` 与 `diff` 支持单侧文件为 `-`。同一条命令不能消费 stdin 两次。

### 搜索与有界浏览

Nocturne 在当前选定的命名空间内提供三种文本检索策略与精确属性过滤，分页均以**去重后的节点**为单位（默认 `--limit 10 --offset 0`）：

```sh
# 1. 默认分词检索（jieba 分词，词元间默认 AND，按 BM25 排序）
nocturne --db memory.db search '记忆 架构' --domain core

# 2. 任意分词匹配（--or：匹配任一分词词元；必须提供查询词）
nocturne --db memory.db search '记忆 架构' --or --domain core

# 3. 字面子串扫描（--contains：显式子串扫描，不用分词或 BM25，适合查找含在长词内的短词）
nocturne --db memory.db search '阿利特' --contains --limit 50 --offset 0

# 4. 顶层字符串属性精确过滤（可单独过滤而不加载 FTS，也可与文本检索组合，多项间为 AND）
nocturne --db memory.db search --attr status=active --attr owner=team
nocturne --db memory.db search '晶型' --contains --attr 'source=GB/T 21372'
```

- **分词检索的字符集与语法边界**：默认分词检索及 `--or` 模式支持中文（`jieba` 分词）以及完整的 Unicode 字母、数字与组合标记（拉丁、希腊如 `β-C2S`、西里尔等，NFC 归一化与 `unicode61` 大小写/变音折叠）。查询按普通词语处理，不支持裸布尔运算符或原生 FTS 表达式语法。
- **`--contains` 字面子串扫描**：默认 FTS 匹配 jieba 切分出的词元。当目标短词（如 `阿利特`）被切分在更长的复合词（如 `阿利特晶型`）内部时，可显式使用 `--contains`。它将整个查询字符串直接作为字面子串，扫描活动正文、URI、disclosure 与当前命名空间触发词（不解析分词或布尔语法，`%` 和 `_` 视为字面字符，ASCII 大小写不敏感）。这是显式扫描模式，**不是全文搜索的自动回退，也不会建立单字索引**；不可与 `--or` 同用。
- **属性过滤规则**：`--attr KEY=VALUE` 仅精确匹配节点 JSON 的顶层字符串字段；键名中的 `.` 视为字面字符而非 JSON 路径，不支持匹配数字、布尔值、数组或嵌套对象，属性内容也不进入全文索引。

#### 搜索范围与精确计数

普通文本输出会说明当前页和是否还有结果；默认只多查看一个去重节点，不为显示总数遍历全部命中。需要精确计数时使用：

```sh
nocturne --db memory.db --json search '矿化剂' --with-total --limit 5
# 只计数，不加载结果正文或生成摘要
nocturne --db memory.db --json search '矿化剂' --with-total --limit 0
```

`--with-total` 的 JSON 为 `{ "total": 40, "offset": 0, "limit": 5, "has_more": true, "results": [...] }`（数字仅为示例）；不带该参数的 JSON 仍是结果数组。`total` 是当前 namespace、domain、属性和查询条件下按节点去重的精确命中数，别名不重复计数，计数与当前页使用同一读快照。越界页仍返回总数；只计数时 `results` 为空，`has_more` 表示 offset 后是否仍有匹配节点。

精确计数需遍历全部匹配候选，`--contains` 还需文本扫描；只为返回页加载结果正文和生成摘要。计数表示**本次查询的规模，不是研究问题的召回率**，不同查询的计数可能重叠。探索“有哪些方法”时应换检索方向、读原文并交代缺口，不要把第一页相关结果当成全貌；日常定位不必计算总数。

浏览与历史版本命令（`<VERSION>` 请替换为 `history` 返回的实际全库版本 ID）：

```sh
# 读取节点（--no-track 不记录访问时间；--no-children 跳过子节点查询）
nocturne --db memory.db read core://项目规范 --no-track --no-children

# 浏览子节点、近期更新、历史版本与按历史版本 ID 恢复
nocturne --db memory.db children core://
nocturne --db memory.db recent --limit 10
nocturne --db memory.db history core://项目规范
nocturne --db memory.db restore core://项目规范 <VERSION>

# 分页索引（支持 --with-total 计数信封）与有界树形浏览（安全硬上限为深度 128、后代 10,000）
nocturne --db memory.db index --domain core --limit 100 --offset 0 --with-total
nocturne --db memory.db tree core:// --depth 2 --limit 200

# 发现命名空间与列出不加载正文的路径清单
nocturne --db memory.db namespaces
nocturne --db memory.db paths --domain core
nocturne --db memory.db paths --all-namespaces
```

### 变更审查、批次与回滚（Review & Rollback）

```sh
# 将多次独立写入归入同一审阅批次（全局 --batch <BATCH_ID> 可放在子命令前或后）
nocturne --db memory.db --batch refactor-notes create core:// --title part1 --file p1.md
nocturne --db memory.db update core://项目规范 --file spec_v2.md --batch refactor-notes

# 筛选待审阅记录、查看批次净差异并按批次整体回滚（--by-batch 表示按批次名查找；不加则传入 changeset UUID）
nocturne --db memory.db review list --status pending --limit 20
nocturne --db memory.db review diff refactor-notes --by-batch
nocturne --db memory.db review rollback refactor-notes --by-batch

# 接受当前命名空间所有待审阅变更（加 --all-namespaces 可扩至全库）
nocturne --db memory.db review accept --all
```

- **`--batch <BATCH_ID>` 与 `--by-batch`**：
  - 全局 `--batch <BATCH_ID>`：为当次写入打上命名空间级审阅批次标签。**它仅用于审阅归组，不会把多个进程或多次命令合并为同一个数据库写事务**。
  - `review diff/accept/rollback <ID> --by-batch`：布尔开关，指示 `review` 将 `<ID>` 解析为批次名而非单条 changeset UUID。批次 diff 按节点 UUID 配对，直接对比批次开始前与结束后的净正文差异，不展示中间过渡版本（`--json` 仍保留完整行快照数组）。
- **回滚冲突保护（无强制覆盖）**：若目标变更之后存在后续冲突编辑，或节点仍被其他存活路径/跨命名空间引用，回滚会**整体中止并返回冲突错误（退出码 `4`，`reason: "rollback_conflict"`）**，绝不覆盖后续修改，也不会只回滚批次的一部分。系统**不提供 `--force` 绕过机制**；请先按需撤销后续 changeset（错误 JSON 中的 `blocking_uris`、`related_changesets`、`foreign_references` 会指出当前命名空间可见的阻塞项，同时隐藏其他命名空间的私有详情），或从一开始就将关联写入放入同一批次。普通的 `read` 访问记录不会阻碍回滚。

### 路径管理、别名与无路径节点（Orphans）

```sh
# 修改最后一段路径名（子树随之移动）与跨父路径/跨域移动（目标父路径需存在）
nocturne --db memory.db rename core://项目规范 架构规范
nocturne --db memory.db move core://架构规范 archive://架构规范

# 为同一节点添加带独立边元数据的别名路径（alias <SOURCE> <DESTINATION>），或移除单条路径
nocturne --db memory.db alias archive://架构规范 notes://规范 --priority 2
nocturne --db memory.db unlink notes://规范

# 发现、查看并恢复在所有命名空间均已无路径的节点（<MEMORY_ID> 取自 orphans list 输出）
nocturne --db memory.db orphans list
nocturne --db memory.db orphans show <MEMORY_ID>
nocturne --db memory.db orphans restore <MEMORY_ID> core://recovered_spec --priority 1
```

- `rename` 与 `move` 仅移动当前命名空间下实际存在的 URI 子树，操作前会预检循环引用与路径碰撞；所有预设中属于当前命名空间的 `boot` 引用会同步重写，不影响其他命名空间的配置。
- `orphans` 仅针对**在任何命名空间都没有路径**的节点（仍有路径节点的旧版本请用 `history` 或 `deprecated` 查看）。出于命名空间隐私隔离，默认仅列出最后一次路径移除记录属于当前命名空间的节点；若需检查或恢复无审计来源的导入孤儿节点，或有意跨命名空间恢复，请显式加上 `--all-namespaces`。

### 目录批量导入与可读导出

```sh
# 预检目录导入（--dry-run 不写入数据库，输出完整路径映射与问题汇总）
nocturne --db memory.db import-dir ./notes --into core:// --sanitize --priority 1 --attrs-file attrs.json --dry-run

# 在单个可回滚事务中执行目录导入
nocturne --db memory.db --batch import-notes import-dir ./notes --into core:// --sanitize --priority 1 --attrs-file attrs.json

# 导出整个命名空间；也可在 export-dir 后传 lit:// 或 lit://docs 缩小范围
nocturne --db memory.db export-dir --output ./readable-export

# 导出单个分支，并附带属性、边配置及共享关系的 manifest
nocturne --db memory.db export-dir lit://docs --output ./docs-export --with-metadata

# 在新库中预检并恢复快照（保留 URI，写入当前 namespace）
nocturne --db restored.db init
nocturne --db restored.db import-dir ./docs-export --manifest ./docs-export/nocturne-manifest.json --dry-run
nocturne --db restored.db import-dir ./docs-export --manifest ./docs-export/nocturne-manifest.json
```

- **`import-dir`**：递归导入 UTF-8 `.md` 和 `.txt` 文件（目录生成空正文节点，跳过并报告隐藏项、符号链接与其他文件类型，`README` 无特殊待遇）。`--sanitize` 保留合法 Unicode 名称字符和原有的 `-`/`_`，将连续的其他字符（空格、点、标点、括号、引号、符号等）替换为 `-`，并仅修剪由替换产生的首尾 `-`（保留原有的合法 `-`/`_`）。**导入绝不覆盖或合并已有路径或清理后产生的碰撞**；只要存在任何无效项或冲突，实际导入即整体拒绝（返回问题计数与最多 3 条样本，完整列表通过 `--dry-run --json` 查看）。
- **来源路径**：普通导入可加 `--source-path-attr source_path`，在同一事务内为每个新文件和目录写入顶层字符串属性，保存相对输入根的原始路径（含文件扩展名，分隔符为 `/`），无需逐条写回。该路径是本次输入的名称，不是外部预处理前的名称；不保存机器绝对路径，不修改正文。不传选项时行为不变。字段名不能为空，也不能与 `--attrs-file` 中已有字段重名（包括 `null`）；预览显示字段名和每项来源路径。该选项与 `--manifest` 互斥，manifest 往返会保留已有来源属性。
- **`export-dir [URI]`**：省略 URI 时导出当前命名空间；`lit://` 选择整个域，`lit://docs` 选择该路径及其后代，按路径段边界匹配，不扩散到其他别名或命名空间。在同一只读快照中逐篇写出 `domain/path.md`，保留正文原字节，先写同级临时目录再原子发布，不覆盖已有目标，不记录访问或产生 changeset。
- **`--with-metadata`**：额外生成 `nocturne-manifest.json`，保存选中范围的 URI、节点属性、边的 priority/disclosure 及共享关系。配套的 `import-dir --manifest <FILE>` 预检文件引用、元数据和目标路径后，在单个可回滚事务中恢复，分配新的数据库 ID。空正文目录节点的属性也保留；`parent.md` 和 `parent/` 通过 manifest 对应到同一节点。导出根之外的父路径需在目标库中预先存在。
- **manifest 导入**：保留原 URI，使用当前 `--namespace`；不与 `--into`、`--sanitize`、`--priority`、`--disclosure`、`--attrs-file` 或 `--source-path-attr` 混用。manifest 及其引用文件必须位于导出目录内，禁止越界路径或符号链接。普通目录导入仍按文件布局命名，不自动读取 manifest。
- **快照范围**：带 manifest 的导出用于当前内容和元数据交换；历史版本、审计、触发词和启动预设由整库 `backup` 保存。普通无 manifest 导出仅含正文与目录结构；有正文的父节点再次普通导入时仍会产生同名冲突。

### 启动记忆（Boot）与预设（Preset）

```sh
# 配置并读取当前命名空间的启动记忆（--no-track 只读加载，不更新访问时间或写日志）
nocturne --db memory.db boot set core://architecture
nocturne --db memory.db boot list
nocturne --db memory.db boot --no-track
nocturne --db memory.db boot

# 管理全库命名的启动预设（boot.json 形如 {"": ["core://agent"], "work": ["core://spec"]}）
nocturne --db memory.db preset create work --file boot.json --activate
nocturne --db memory.db preset list
nocturne --db memory.db preset show work
nocturne --db memory.db preset duplicate work writing
nocturne --db memory.db preset activate writing
```

- 预设是全库共享的启动 URI 映射表（不是访问控制或记忆副本），最多一个处于激活状态，变更均可审计、可回滚。
- 执行 `boot` 时仅读取活动预设中当前 `--namespace` 对应的条目（绝不跨 `""` 命名空间静默回退）。若任何配置的 URI 缺失，`stdout` 输出完整的 `loaded`/`failed` 报告，`stderr` 输出 `not_found` 错误 JSON，并以退出码 `3` 结束。

---

## 治理诊断、维护与安全规程

### 治理诊断与加权随机复习

```sh
# 诊断过期记忆、子节点过多、断裂父路径、同父重复别名与最大正文
nocturne --db memory.db diagnostic --domain core --days-stale 30 --max-children 10 --min-bytes 2048

# 按未访问天数与边优先级加权抽取一条记忆复习（--no-track 不更新访问时间）
nocturne --db memory.db random --domain core --no-track
```

- **过期阈值规则**：节点按 `coalesce(last_accessed_at, created_at)` 计算未访问天数；`priority` 为 `0`、`1`、`2`、`3` 的记忆分别在 `3`、`7`、`14`、`28` 天未访问后标记为过期，此后优先级每加 1 阈值翻倍（上限 36,500 天）。缺失优先级或 **`priority = 999` 表示回退使用 `--days-stale`（默认 30 天），而不是“永不过期”**。结果包含 `stale_nodes`、`crowded_nodes`、`broken_parent_routes`（缺失直接父路径的子路由，区别于无路径的 `orphans`）、`duplicate_aliases` 与 `longest_memories`。
- **分域治理建议**：可将整理后的核心知识与原始文献放在同一命名空间的不同域（如 `core://` 与 `lit://`），检索时在当前命名空间内跨两个域搜索，治理时用 `diagnostic --domain core` 仅检查核心域，无需为了隐藏过期提醒而把所有文献的优先级调大（调大优先级会同时降低 `random` 抽样权重：`max(unvisited_days, 0.5) * max(0.5^priority, 1e-12)`）。

### 废弃版本清理（Purge）与日志清理（不可逆操作）

```sh
# 列出当前命名空间可见的历史/废弃版本
nocturne --db memory.db deprecated

# 预览并执行单个废弃版本的永久物理清理（<MEMORY_ID> 取自 deprecated 输出；不加 --apply 仅预览）
nocturne --db memory.db purge <MEMORY_ID>
nocturne --db memory.db purge <MEMORY_ID> --apply

# 预览并清理早于 30 天的访问日志（--keep-days 0 选择全部日志；不加 --apply 仅预览）
nocturne --db memory.db logs clear --keep-days 30
nocturne --db memory.db logs clear --keep-days 30 --apply
```

> **安全警告**
> - `purge` 与 `logs clear` 默认均为**只读预览**，必须显式传入 `--apply` 才会执行删除。
> - 这两项操作是**不可逆的维护操作**，不会生成可回滚的 review changeset，执行前请务必先运行 `backup`。
> - `purge` 只能删除废弃版本（`deprecated`）。若待审阅（pending）的变更引用了该节点、待修复版本链或同命名空间的相关批次，清理会被拒绝。若清理会影响共享节点或其他命名空间的审计、触发词、日志，必须显式传入 `--all-namespaces`（但该参数绝不会绕过未决依赖检查）。若无路径节点仍有多个历史版本，需先清理较旧版本，最后清理记录了最后一次 `unlink` 归属的版本。
> - 逻辑删除不会对磁盘底层字节做安全擦除，也不会自动缩小 SQLite 文件体积（需配合 `vacuum`）。

### 容量统计、健康检查、物理压缩与独立文本比对

```sh
# 全库只读容量报告（记录数、正文与审计逻辑字节、日志保留期、SQLite 页与 WAL 分配）
nocturne --db memory.db stats

# 重建派生搜索索引、物理压缩空闲页、校验数据库与 FTS5 外部内容倒排索引一致性
nocturne --db memory.db reindex
nocturne --db memory.db vacuum
nocturne --db memory.db doctor

# 无需数据库的独立 UTF-8 文本比对（支持 --old/--new 或 --old-file/--new-file，单侧可用 - 读 stdin）
nocturne diff --old-file old.txt --new-file new.txt
```

- **`stats`**：始终报告全库范围的只读统计（不受 `--namespace` 限制）。文本大小按 UTF-8 字节计，存储占用按 SQLite 页计；节点数排除 ROOT，无路径节点的大小包含所有保留版本。普通输出只展示统计结果，计量口径见 `stats --help`，JSON 保留 `semantics` 字段。统计不执行清理，也不估算删除后可回收的空间。
- **共享正文存储**：`stats.contents` 区分实际存储、仅审计引用和无引用正文的数量与字节；`doctor` 会报告无引用残留。正文存放在不可变的内容表中，由版本和审计快照共同引用；检索通过关联读取正文，不再保存一份完整搜索正文。审查输出按需还原完整正文，接受或回滚审查不会自动丢弃其引用内容。数据库格式以根 `Cargo.toml` 为准；旧格式仍需保留对应的旧版程序使用，新版不会自动转换。

- **`vacuum`**：依次执行 WAL checkpoint、`VACUUM` 物理压缩与二次 checkpoint，回收空闲页以缩小数据库文件，不删除任何历史版本或审计记录。运行前请停止其他并发读写进程，并预留约数据库大小两倍的临时磁盘空间（若 checkpoint 被并发连接阻塞会报 `conflict`）。

### 备份与恢复安全规程

```sh
# 在线创建一致性备份，并对备份文件运行完整性检查
nocturne --db memory.db backup --output memory.backup.db
nocturne --db memory.backup.db doctor
```

1. **禁止裸拷活动库**：始终使用 `backup` 命令备份，不要在遗漏 `-wal` 的情况下直接复制正在使用的 SQLite 主文件。备份目标所在文件系统需支持硬链接（用于原子发布完整备份且绝不覆盖已有目标文件）。
2. **安全恢复步骤**：停止所有写入进程 -> 将当前数据库主文件及其 `-wal`、`-shm` sidecar 文件作为一套独立恢复集整体移走保留 -> 将备份文件复制到新路径 -> 先对新路径执行 `nocturne --db <NEW_PATH> doctor` 确认无误，再切换业务 `--db`。**切勿将不同数据库的主文件与 `-wal`/`-shm` sidecar 混用**。
3. **格式边界**：备份须使用当前支持的数据库格式，具体格式版本见 `Cargo.toml` 的 `package.metadata.nocturne.schema-version`；恢复备份不提供旧版数据库升级能力。

---

## 自动化、输出格式与退出码

- **输出流约定**：默认输出便于人类阅读的文本；传入 `--json` 时，成功结果以单个 JSON 值（非 JSONL）写入 `stdout`。无论是否指定 `--json`，**任何非零退出（1–5）始终向 `stderr` 写入结构化 `{"error": ...}` JSON**（包含稳定的 `code` 字段，以及冲突场景下的细分 `reason`）。
- **诊断报告共存约定**：`doctor`（退出码 `1`）、`import-dir --dry-run` 发现校验问题（退出码 `2`/`4`/`5`）以及 `boot` 存在缺失 URI（退出码 `3`）在向 `stderr` 写入错误摘要 JSON 的同时，仍向 `stdout` 输出完整的诊断/预检报告。
- **自动化调用建议**：脚本与 Agent 调用时请始终传入 `--json`、显式指定 `--db` 与 `--namespace`，并**先检查退出码**：非零退出即可直接从 `stderr` 读取错误 JSON，诊断命令还可从 `stdout` 读取完整报告。

| 退出码 | 错误码 / 场景 | 含义说明 |
| :---: | :--- | :--- |
| `0` | — | 执行成功 |
| `1` | `integrity` | `doctor` 发现数据或索引不一致（完整报告在 `stdout`，错误摘要在 `stderr`） |
| `2` | `invalid` | 参数、缺失 `--db`、URI、非 UTF-8 输入无效，或 `import-dir --dry-run` 发现输入校验问题 |
| `3` | `not_found` | 所选命名空间中未找到目标，或 `boot` 存在加载失败的引用（`loaded`/`failed` 报告仍在 `stdout`） |
| `4` | `conflict` | 状态或并发冲突：`version_mismatch`、`patch_no_match`、`patch_ambiguous`、`rollback_conflict`、路径占用或导入碰撞等 |
| `5` | `storage` | 底层 SQLite 存储错误或真实文件系统 I/O 失败（含 `import-dir --dry-run` 遇到真实文件 I/O 错误） |

---

## 开发、验证与仓库结构

### 开发检查与可移植验收

日常开发检查：

```sh
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
```

完成 `cargo build --locked --release` 后，可通过内置的 Rust `xtask` 运行验收：

```sh
# 便携式验收：将二进制复制到仓库外临时目录，清空 PATH 后验证端到端工作流（CI 在三平台自动执行）
cargo xtask portable-smoke
cargo xtask portable-smoke --binary /path/to/nocturne

# 大库规模验收（可选）：使用 1,578 篇文档（约 110 MiB 正文）与 59 个整理节点的合成语料验证性能与一致性
cargo xtask large-library-smoke --binary /path/to/nocturne --output results.json
```

如需锁定依赖解析，可执行 `cargo run --locked -p xtask -- portable-smoke`。验收任务的覆盖范围与测量方式见 [xtask 验收说明](xtask/README.md)。普通成功流程由 `portable-smoke` 验证；回归测试聚焦具体失败风险，保留理由写在[测试源码](tests/)的注释中。

### 仓库结构

仓库根目录为单一 Cargo workspace，包含默认根包 `nocturne`（提供 `nocturne` 库与 `nocturne` 可执行文件）以及开发任务包 `xtask`：

- `src/`：核心存储引擎、内嵌初始 SQLite schema（`src/schema.sql`）、FTS5/jieba 检索、图路径与 CLI 实现。
- `tests/`：聚焦数据安全、命名空间隔离、索引一致性与进程/I/O 边界的回归测试集，每项测试在源码注释中说明保留理由。
- `xtask/`：纯 Rust 编写的跨平台可移植验收与合成大库测试工具（详见 [xtask/README.md](xtask/README.md)）。
- `skills/`：可独立安装的通用 Agent Skills，包含日常记忆操作和只读优先的记忆审计流程。

本项目基于 [MIT 许可证](LICENSE) 开源，保留原始版权声明。
