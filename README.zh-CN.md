<p align="center">
  <img
    src="assets/readme/hero.svg"
    width="100%"
    alt="A3S CLI runs one coding workspace in the terminal, with a reviewed cognitive-package path on main"
  />
</p>

<p align="center">
  <strong>Language / 语言:</strong>
  <a href="README.md">English</a> ·
  <a href="README.zh-CN.md">中文</a>
</p>

<p align="center">
  <strong>在终端中使用代理进行构建。通过经过审查的版本化包扩展同一主机。</strong>
</p>

<p align="center">
  <a href="https://github.com/A3S-Lab/CLI/actions/workflows/ci.yml"><img src="https://github.com/A3S-Lab/CLI/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
  <a href="https://crates.io/crates/a3s"><img src="https://img.shields.io/crates/v/a3s.svg" alt="Crates.io version" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-f0b65b.svg" alt="MIT license" /></a>
</p>

<p align="center">
  <a href="#quick-start">快速入门</a> ·
  <a href="#cognitive-packages-gated-preview">认知包</a> ·
  <a href="#a3s-code">A3S 代码</a> ·
  <a href="#component-lifecycle">组件</a> ·
  <a href="#release-readiness">准备</a> ·
  <a href="#development">开发</a>
</p>

> [!重要]
> **A3S 0.14.0 — 2026 年 9 月 3 日。** 该存储库是规范的 CLI
> GitHub 档案、crates.io 和 Homebrew 的发布源； A3S 单一存储库
> 为 0.11 客户端发布字节相同的兼容性中继。发布
> 包括 Code Core 8.1.0 的生成精确能力运行时、异步会话拥有的语义
> 检索、电源管理的本地 MiniLM/ONNX 配置以及默认拒绝
> 离线`local-workspace`自动化边界。现在托管的 macOS 沙箱
> 保留具有超过 4,096 个多链接文件的源树，传输
> 按文件分析安全带配置文件，并将大型文字路径集编译为精确的路径集
> 有限正则表达式尝试其编码匹配器保留在 Seatbelt 解析器下方
> 上限，因此 JavaScript 生成和 Intel 编译仍然受到限制。
> 认知包托管是
> 仅作为门禁预览包含在内，并且不可用的提供程序继续失败
> 关闭。

## 一个 CLI，一台代码主机

`a3s` 是 A3S 开发者平台的总括命令。基础安装
包含 A3S 代码。其他本机产品和 A3S 使用功能保持其
自己的发布和生命周期边界。

```text
a3s
├── code        interactive or non-interactive coding agent
├── plugin      reviewed cognitive-package lifecycle (gated preview)
├── use         Browser, Office, OCR, and installed Use capabilities
├── compose     multi-service applications delegated to A3S Box
└── components  install · upgrade · inspect · repair · uninstall
```

|切入点|现任职务 |
| ---| ---|
| `a3s code` | TUI、受管工具、持久会话、内存、研究、资产创作和本地 Flow 执行。 |
| `a3s plugin …` |通过门控预览搜索、查看、安装、升级、启用、禁用和卸载认知包。 |
| `a3s use …` |将浏览器、Office、OCR、Box 和扩展功能委托给 A3S 使用。 |
| `a3s install …` |管理注册的A3S产品和委托使用包；它不是通用操作系统包管理器。 |

### 0.14.0 中证明了什么

发布源对重要的边界进行了回归覆盖
包主机：

|证据|锻炼什么 |
| ---| ---|
|代码核心 8.1.0 TUI 集成 | TUI 解决了确切的 Code Core 8.1.0 和 Search 3.1.0 修订版，执行本机沙箱和 Moli 支持的 Web 搜索策略，并通过 SDK 适配器表面保持核心功能目录可用。 |
|独立的 Moli 档案 |发布矩阵从不可变的代码清单中下载 Moli 1.1.1，验证其摘要和目标可执行格式，将 `moli/` 注入每个 macOS、Linux 和 Windows 存档，并在上传前检查最终存档成员。 |
| Linux、macOS 和 Windows CI | Linux 运行完整的测试、lint、安装程序和发布构建门； macOS 运行本机安装程序/TUI 回归和发布版本； Windows 运行其安装程序矩阵和发布版本。 |
|受监管的本地编码政策 | `local-workspace` 配置文件需要非交互式 Auto，仅在 A3S 拥有的本机沙箱通过其探测后才公开 Bash，保留工作区读取、代码智能、有界编辑、结构化本地 Git 和受控委派，并拒绝主机升级、下载、运行时、知识、托管工具、MCP 和未知动态工具。默认情况下，Web 读取仍然被拒绝，并且只能由独立的 `--web-search enabled` 边界允许。嵌套任务和技能运行继承相同的实时检查器、沙箱和默认拒绝的可序列化回退。 ||原生沙箱边界 | Linux、macOS 和 Windows 发布门针对工作区写入、受保护的控制路径、网络拒绝、进程清理和后端功能探测运行编译的沙箱，而无需 Node.js 或 npm 支持有效负载。 |
|已审核使用授权桥|委托计划者发出一份与提供者无关的、不受约束的草稿。然后，主机在策略审查之前绑定来自签名规划包、显式运行时分配和当前提供者功能的确切授权和提供者证据，重复与最终权限的绑定，并拒绝任何提供者、构建、功能、语义、执行或权限漂移。真实签名的 schema-v3 包在进程内使用图内保留伞式操作 ID、规范计划、依赖锁、授予快照、规划捆绑包、审查的提供者证据和确认； apply 永远不会启动子 `a3s` 突变。 |
|受保护的托管工作区主机 |协议 v6 明确规划签名包的启用/禁用转换，将用户确认绑定到其操作 ID 和摘要，并通过现有主机应用请求进行应用。规划证据、应用意图、能力切换和结果在主机娱乐中得以保留；过时的生成、请求或摘要替换、包字节更改和依赖关系图更改无法关闭。具有权限的工具回归证明，缺少确认不会造成应用意图或生命周期突变。 || TUI 第一帧延迟 |阻塞的 Evolution 阅读器、无响应的配置 MCP 和 25,000 个文件工作区证明可见加载框架先于可选功能工作和存储库发现。 PTY 回归强制执行三秒硬上限。之前的 12 轮发布基准测试在同一 macOS 主机上测得的中位数为 99.270 毫秒，p95 为 139.452 毫秒。 |
| TUI 首次使用集成 | Linux、macOS 和 Windows 将独立构建的 A3S Use 版本打包为平台本机存档，在代码保持响应的同时安装它，容忍有限的一次性可执行文件扫描，并证明附加的注册表修订在第一个模型转动之前是可见的。 |
|原子使用运行投影 |驻留主机使用一种类型化的使用注册表快照和游标，然后将经过验证的托管 MCP 服务器、技能、提供者合格的运行时工具任务、摘要绑定的不可查询的知识表面准备情况、依赖关系封闭的本地流和绑定的 UI 文档作为一批核心会话发布。每个承认的运行或主机句柄都会获取新的非克隆使用快照租约。当 N+1 发布时，N 仍然可用，失败的准备使 N 可见，并且预计值永远不会进入可变的兼容性注册表。 ||范围一次性使用运行时 |普通 Code Exec 仅重用已经准备好的 Use 安装，并且从不隐式安装它。所需的桌面调用协商 `scoped-v1`，可以执行策略授权的首次使用设置，在模型退出之前冻结一个原子托管 MCP/技能/运行时任务/UI 生成，并返回准确的代码目录、使用光标、表面计数和任务计数/摘要证据。一个进程拥有的插件管理器提供精确生成的任务调度和可信的不透明 HTTP MCP 路由解析，直到会话关闭；接下来是有界运行时/网关关闭。缺少或格式错误的所需证据将无法关闭，而内置 MCP、兼容性知识、流程和插件管理器演示文稿仍不在此范围内。 |
|管理 OKF 知识 |真实的签名包测试涵盖安装、持久 SQLite/FTS5 投影、进程重启、精确代升级、陈旧代撤回、引用搜索、卸载、全范围使用核算、配额释放、逻辑删除和物理页回收。范围本地测试还涵盖完整性审核、非覆盖备份、离线验证和确认 FTS 修复。受监视的注册表将相同的只读搜索工具热插入 TUI 会话中；每个接受的查询都通过后端搜索和修订验证保存准确的包生成注册表租约。 ||主机绑定的运行时生命周期 |真实签名的 OCI 工具任务回归证明，缺少的主机分配在存档下载之前失败，注入的提供程序仅由主机选择，并且构建漂移在安装突变之前失败。 Linux/macOS/Windows monorepo 门为该受信任的主机测试提供独立构建的、精确修订的 `a3s-use` 可执行文件，因此计划和切换后能力证据跨越真实的进程边界，同时运行时和授予权限仍注入到插件管理器中。 Schema-v4 任务绑定保留无参数的经过审查的运行时模板和确切的提供者/授予证据。共享管理器调度程序在重新启动后重新连接该提供程序，仅派生每次调用身份和有界 argv，拒绝隐藏代，并通过捕获和清理来保留注册表租约。仅当指定的已审核提供程序存在时，功能快照 v2 才会将精确任务作为保守的 `use_tool_*` 工具投射到 TUI 和范围内的 Code Exec 会话中；提供者缺席会产生警告并且没有工具。升级并禁用在更换前撤回旧的动态工具。受信任的用户或显式 ACL 可以为 Linux 上的版本支持工具任务组成共享 Box 提供程序；添加其私有网关块会将相同的提供程序分配给工具服务和可流式 HTTP MCP。遗漏保持故障关闭状态，并且没有提供者后备。 ||共享托管执行边界 |代码将包主机组合委托给共享的 A3S Use 托管工厂。运行时服务将精确类型化的环回端点发布到一个持久的私有网关中。工具路线通过审查计划健康状况； MCP 路由通过返回的端点完成标准初始化/初始化。目标身份可从最终接收中重建，重新启动可恢复相同的路由，而退出是网关准入关闭/耗尽、运行时停止、精确网关移除、然后运行时移除。 CLI/TUI 退出显式关闭侦听器。 Linux gateway 通过生产 Box 映射、运行时状态、网关重启、保留代路由、排出、精确删除和零残留检查来运行真正的 N/N+1 Tool 和 MCP 流程。它还独立地终止每个提供程序进程，并证明使用新端点、同级隔离、重复 MCP 初始化和最终零残留清理来相同运行时替换确切的过时网关绑定。非 Linux 提供商和跨平台恢复矩阵保持开放。 |

这些测试支持预览声明。它们不会取代释放门
[发布准备](#release-readiness)。

## 快速开始

使用一个命令安装最新的公共稳定版本：

```bash
# macOS or glibc Linux (x86_64 / arm64)
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/A3S-Lab/CLI/main/install.sh \
  | A3S_MODIFY_PATH=1 sh
```

```powershell
# Windows x64 — PowerShell 5.1 or newer
$env:A3S_MODIFY_PATH = '1'
irm https://raw.githubusercontent.com/A3S-Lab/CLI/main/install.ps1 | iex
```

安装程序在安装期间比较两个官方版本存储库
当前迁移，选择较新的稳定SemVer，验证GitHub发布
SHA-256，拒绝不安全的存档成员，验证`a3s --version`，并激活
二进制、捆绑的特定目标 Moli 运行时和可选的 WebView
作为一项可恢复操作的伴随。他们从不使用 `sudo` 或 UAC。省略
`A3S_MODIFY_PATH=1`离开
shell 配置文件和用户路径不变。

包管理器安装仍然可用：

```bash
# macOS or Linux with Homebrew
brew install A3S-Lab/tap/a3s

# Any platform supported by the Rust toolchain
cargo install a3s --locked
```

上述独立安装程序支持 Intel macOS 12。当地人
macOS 沙箱使用操作系统的内置 Seatbelt 边界，并且不会
不需要 Node.js 或 npm 运行时。

在终端中启动：

```bash
a3s code
```

第一次 `a3s code` 启动在未配置时创建 `~/.a3s/config.acl`
存在。使用以下命令检查选定的 ACL 配置：

```bash
a3s config path
a3s config show
a3s config validate
```

A3S 0.14.0 包含门控认知包主机。准备并检查 A3S
当首次使用安装不合适时显式使用：

```bash
a3s install use --source release
a3s use doctor --json
a3s use capabilities --json
```

`--offline` 和 `A3S_NO_AUTO_INSTALL=1` 是严格的禁止下载边界。

## 认知包：门控预览

认知包是 A3S 拥有的类似 npm 的 SemVer 分发单元
使用。它具有稳定的 `<publisher>/<name>` 身份、ACL 清单、必需的
自述文件、可选包依赖项以及六个表面的任意组合
合同：

```text
acme-research/
├── a3s-use-extension.acl   identity · version · dependencies · surfaces
├── README.md               required package documentation
├── tools/                  executable Tasks or long-lived Services
├── releases/               content-bound Tool and MCP descriptors
├── flows/                  A3S Flow TypeScript workflow sources
├── skills/                 SKILL.md files and supporting content
├── ui/                     integrity-bound static assets
└── okf/                    Open Knowledge Format bundles
```

完整的包生成（而不是单个文件）是安装，
升级、启用、禁用和卸载单元。依赖项先安装
依赖项，未使用的依赖项按相反顺序卸载，一次成功
割接发布了新一代的能力。

### 按表面划分的主机准备情况

该封装格式接受所有六个表面。当前代码主机不
假设每个执行适配器都已准备就绪：

|表面|创作于 `main` |仍然有门禁|
| ---| ---| ---|
| **技能** |内容验证、全代核心会话发布以及每次承认的运行的新的精确使用快照租约。 N 仍然固定在 N+1 发布和生命周期消耗中。 | — |
| **用户界面** |有界 UTF-8 活动 HTML/CSS/JS 被重新验证，复制到无路径 Core `UiBinding` 中，并以其规范技能、合格的运行时工具任务、托管 MCP 和依赖封闭流依赖项以及确切的 Use 租约以原子方式发布。 N 个句柄在 N+1 中保留 N 个字节。 |经过审查的渲染器。仅允许来自同一个包生成的依赖关系；不可用 工具、MCP 或流程依赖项使候选批次失败，而不推进当前目录。 |
| **MCP** |每个扩展表面都保留其规范 ID、多重性、激活、准确的生命周期身份、文件证据和特定于传输的启动器/就绪证据。代码重新验证包文件，仅解析包限制的 stdio 可执行文件或受信任的无凭据环回运行时路由，并将 Core `McpBinding` 值暂存在与其 Flow/UI 依赖项相同的原子批次中。内置浏览器/OCR 路由保留其现有的兼容性所有者。 | Real Box MCP 进程终止和保留一代产品 E2E、非 Linux 提供商组成以及跨平台恢复矩阵。 || **工具** |验证非交互式本机任务生命周期、从持久 schema-v4 收据中精确生成运行时调度，并在与技能/UI 相同的核心批次中将 TUI 投影视为保守的 `use_tool_*` 值。投影携带准确的包/清单摘要、生命周期生成、范围、表面和提供者 ID；调用仅接受有界 argv 并且从不查阅当前分配。受信任的 Linux 主机 ACL 可以显式地将共享 Box 提供程序分配给版本支持的工具任务，并通过其私有网关分配长期存在的工具服务。如果不发布工具，遗漏和不受支持的平台将无法关闭。升级并禁用替换或撤销整个原子生成，无需兼容性注册表双重写入。 |工具服务仍归 Gateway 所有； real Box Service 进程终止资格、非 Linux 提供商组成以及 real 提供商跨平台卸载/升级恢复矩阵仍然开放。 || **A3S 流程** |无依赖项、工具依赖项、MCP 依赖项和 OKF 依赖项本机 TypeScript 流程经过源代码验证、摘要阶段、预检，并作为驻留原子批处理中的精确 `FlowBinding` 值发布。工具、MCP 和不可查询的知识表面边缘仅在同一包生成内解析；编译失败、适配器准备、缺少 OKF 证据或锁定等待取消会使当前目录保持不变。持久的本地运行、状态和历史保留其现有所有者。 |分布式安置、自动恢复和生产保留保持开放。 |
| **OKF** |范围感知的 SQLite/FTS5 暂存和升级、收据核算的范围配额、有界代和逻辑删除、物理清理、持久绑定、重新启动恢复、完整性审核、派生索引修复、版本化备份/离线验证、监视 TUI 投影、通过 `use_knowledge_search` 进行引用检索，以及参与生命周期消耗的精确发布生成查询租约。一个包表面的精确投影也跨范围规范化为常驻原子批次中的无路径、不可查询的核心`KnowledgeSurfaceBinding`。 |显式的包绑定认知会话选择、协调恢复、权限恢复、备份轮换、托管回滚语义、分布式知识放置和完整的跨平台发布矩阵。 |

当适配器或证据不可用时，所需的表面无法关闭；
他们从不默默降级到其他提供商。

常驻 CLI 提供经过验证的托管 MCP 服务器、技能、
提供者限定的运行时工具任务，摘要绑定的知识面
准备就绪、依赖封闭的本地流程以及通过一个的无路径 UI 绑定
核心原子目录剪切。用户界面和流程
依赖项使用规范的 Use 表面 ID 并需要版本化的 Use
完整性标记，包括空依赖集；楼主从来没有
重新解析包 ACL 或从静态资产推断权限。一个工具-或
依赖于 MCP 的 UI/流程和依赖于 OKF 的流程仅针对某个值进行解析
来自同一个完全相同的包生成。内置浏览器/OCR MCP 包装器和
`use_knowledge_search` 在该原子之后保留类型化兼容性所有者
出版；可查询性不是从知识表面准备情况推断出来的。
任何缺失的依赖项都会导致候选人失败
批处理而不推进当前目录。 OCR是一种非租赁的
主机内置覆盖，而其 ONNX 运行时 ABI 不同于可选的本地
嵌入 ABI；其经过验证的技能仍然通过原子目录发布。

### 审查生命周期

配置明确信任的注册表后，可以搜索元数据
无需下载包档案。突变创建了一个不可变的计划
在他们改变活跃一代之前：

```bash
a3s plugin search research
a3s plugin inspect acme/research

# Interactive review and apply
a3s plugin install acme/research --channel stable
a3s plugin upgrade acme/research
a3s plugin disable acme/research
a3s plugin enable acme/research
a3s plugin uninstall acme/research

# Non-interactive two-step apply
a3s --output json plugin disable acme/research --dry-run
a3s --output json plugin apply <operationId> \
  --plan-digest <planDigest> \
  --yes
```

CLI、`/packages` 和管理 MCP 是一个演示适配器
A3S 使用`PluginManagerService`。搜索、检查、安装状态、不可变
因此，规划、持久应用和重播会暴露相同的使用拥有的类型
合约和规范的 `user/current` 范围。 MCP标准发布
精确的十工具管理器-v4 库存；其应用工具仍处于故障关闭状态
因为 MCP 请求永远不会被视为可信用户确认。
交互式 CLI 和 TUI 审查另外共享一个确定性只读
精确的不可变包络的投影。它命名了计划标识并
摘要、候选/先前包图、选定注册表或保留安装
来源、每次转换和完整的权限上限、提供者和
工作空间影响证据，以及确切的确认边界。标准型
机器 JSON 响应保持不变。

每个生命周期突变都需要当前的 Catalog-v3 证据和
完整的认知包锁定。共享服务持续存在
突变前的`PluginHostPlanResult`，仅接受其操作ID，
`planDigest`，并在申请时准确确认。代码注入注册表访问权限，
ACL策略、生命周期/运行时/UI组成、确认边界；
单独使用拥有包解析、计划持久性、突变和重放。
包内容无法选择运行时提供程序或预绑定主机权限。

对于锁定图，A3S Use 派生出候选生命周期代数并
授予/提供者证据，主办方根据整个计划评估政策，
使用以最终权威重新生成证据。规划被拒绝
提供商身份/构建、功能、工作负载语义、执行、
权威、范围或修订偏差。 v3操作记录持久化
规范的注册表源修订版、规划包、精确的拨款快照、
并在计划和确认的同时完成经过审查的提供者证据。

已审核的启用使用其自己的 schema-v2 持久计划记录。它存储了
安装已签名的规划包、精确的拨款快照以及包到提供商
世代——而不是进程本地运行时客户端。应用重新连接配置的
提供者，并且必须复制经过审查的证据。禁用携带一个空
候选者选择并让 A3S 使用从确切的绑定收据中退出；
重新启用从保留的捆绑包中重建激活。这仍然有效
管理器重新启动后，不会重新获取注册表目标。

在应用时，插件管理器仅重建冻结在中的注册表身份
锁，重新派生授予、生命周期生成、主机分配，以及
从当前证据中选择运行时，并且需要与
在创建生命周期工厂或下载之前审查提供商记录
包存档。然后它调用 A3S Use in-process
`ReviewedCognitivePackageAuthorizationProvider`。使用必须重现相同的
操作 ID、计划摘要、包转换、影响、状态修订、锁定、
拨款，并在其发生变异之前提供证据。缺少证据，年龄较大
架构，或者未锁定的计划在规划过程中被拒绝；申请没有
子进程突变回退。

Grant 快照绑定到计划的确切规范`user/current`范围
和持久的状态修订。范围、修订、预约束影响/提供商证据、
或者在申请之前最终权限漂移失败。签名的 OCI 工具任务回归
证明缺少分配和更改的提供程序构建在存档之前失败
或生命周期突变，同时恢复已审查的版本仍保留确切的
授予收据而不启动子突变并幂等重播。
升级绑定了确切安装的锁和候选锁；升级和
卸载保留仍由另一个已安装的根图拥有的依赖项。

托管工作区启用和禁用现在使用显式两步协议。
主机保留 `PluginHostEnablementPlanRequest` 及其确切的 plan-v4 或
终端 `NoChange` 结果，然后仅接受现有的摘要绑定
`PluginHostApplyRequest`。确认应用重建
`ReviewedCognitivePackageAuthorizationProvider`，A3S 使用再现
在启用之前的相同计划和格兰特传奇可能会发生变化。许可
因此，包使用相同的准备、切换、耗尽、退出和崩溃
恢复路径作为他们审查的图形生命周期。包字节和
依赖图不改变。

本地 CLI 和 TUI schema-v3 启用/禁用使用相同的审查
两步合同。规划保留完整的用户范围使用范围和
返回带有操作 ID 和规范摘要的 `planned` 或终端
`no-change` 无合成突变同一性。应用重新验证政策，
持久意图之前的生命周期、摘要和准确确认，然后恢复或
仅重播意图后记录的传奇故事。 `a3s plugin apply`接受这些
启用计划以及安装、升级和卸载计划。

在代码 TUI 中，`/packages` 仅在代理空闲时可用。它分页
与 CLI/MCP 相同类型的已安装软件包状态，并显示使用拥有的 `desired` 和
`observed` 分别取值。 Enter 创建一个不进行变异的计划；评论
显示完整的操作 ID、规范摘要、预期包生成、
过期、包图、源、转换、权限、提供者、影响、状态、
以及在 Enter/y 可以应用该确切身份之前的确认证据。向上和
向下滚动完整评论； Esc/n 取消，无身份`NoChange`
刷新而不应用，并且在确认应用时面板保持锁定状态
正在飞行中。 `/plugin` 仍然是一个单独的本地克劳德/法典技能开关
不管理 A3S 使用包。

代码 TUI 观察每个进程一个能力观察者：

```text
verified install   → generation N+1 → ready surfaces appear
reviewed disable   → generation N+2 → callable surfaces withdraw and drain
reviewed enable    → generation N+3 → exact installed surfaces return
verified upgrade   → generation N+4 → old evidence is replaced atomically
verified uninstall → generation N+5 → package surfaces withdraw and drain
```

### 管理 OKF 知识

安装的 OKF 表面被索引为不可变的、不可执行的内容；代码
不会将包粘贴到系统提示符中。提升的投影绑定
确切的用户或工作空间范围、包和表面身份、生命周期
生成、包和捆绑摘要、投影收据和索引摘要。
在范围内只能有一代表面处于活动状态。

当至少一个投影处于活动状态时，当前和新连接的 TUI
会话接收只读 `use_knowledge_search` 工具。禁用或
当没有托管知识剩余时，卸载会删除该工具。查询快照
实时注册表生成，将其投影重复数据删除到精确的包中，
清单和生命周期生成身份，并获取每个相应的
在 SQLite 访问之前发布注册表租约。这些租约仍保留至
后端搜索和最终注册表修订验证，因此接受查询
在切换之前参与生命周期耗尽并阻止上一代
退休直至完成。缺少租约和相互冲突的预测
摘要失败关闭。与替换者重试一次赛车切换
修订；过时的结果永远不会作为当前上下文返回。每一次打击都承载着
它的确切概念路径、源摘要、包生成、投影收据、
和索引摘要。

组成的适配器继承每个完整的使用默认存储策略
用户或工作空间范围：512 MiB 的收据计算的扩展内容，256
保留投影，每个表面 32 代，以及 256 个移除墓碑。
登台以原子方式检查整个范围；收据拥有的移除免费配额，
修剪墓碑、清理 SQLite 并截断其 WAL。操作人员可以检查
通过`a3s use knowledge usage --json`非秘密分配证据；一个
精确的工作空间查询还需要`--scope-kind workspace --scope-id <id>`。

Use 还通过相同的透明方式公开范围限制的操作员命令
`a3s use`代理：

```bash
a3s use knowledge audit --json
a3s use knowledge backup ./workspace.a3s-okf-backup \
  --scope-kind workspace --scope-id workspace/acme --json
a3s use knowledge verify-backup ./workspace.a3s-okf-backup \
  --scope-kind workspace --scope-id workspace/acme --json
a3s use knowledge repair-search-index --yes \
  --scope-kind workspace --scope-id workspace/acme --json
```

审计验证 SQLite、外键、收据、会计、范围身份和
FTS诚信。修复仅在权威后重建派生的 FTS 行
状态通过验证。备份是非覆盖的且其有界清单
数据库摘要可以离线验证，但它既不是注册表
签名也不是整个产品的恢复工件。协调恢复不是
已实施；不支持将快照复制到活动状态。

### 可替换的注册表源

注册表 URL 和信任身份属于主机配置，而不属于主机配置
不受信任的包。可以添加、禁用或禁用镜像或私有源
无需更改解析器代码即可替换：

```bash
a3s registry add packages https://packages.example.org/a3s/ \
  --root-sha256 <root-sha256> \
  --trusted-root ./root.json \
  --yes
a3s registry refresh packages
a3s registry list # copy the current revision before each mutation
a3s registry disable packages --revision <current-revision> --yes
a3s registry replace packages https://mirror.example.org/a3s/ \
  --root-sha256 <mirror-root-sha256> \
  --trusted-root ./mirror-root.json \
  --revision <current-revision> \
  --yes
a3s registry enable packages --revision <current-revision> --yes
```

`state/use/registries.acl`是CLI使用的唯一Registry源文档，
TUI、计划和应用。每个突变都使用修订版 CAS。
安装可以选择带有`--registry-name`的启用源；升级和
卸载仍固定到已安装的来源。保留已安装的收据
固定到提供它们的注册表身份。
更换来源不会重写这些收据；升级失败关闭
直到来源恢复或显式迁移。正式生产
注册表根尚未公开，因此这些命令当前需要源
经营者刻意信任。

## 架构

<p align="center">
  <img
    src="assets/readme/cognitive-hotplug-architecture.svg"
    width="100%"
    alt="Trusted package sources pass through one Plugin Manager and A3S Use graph before Skills, Flow, OKF, and provider-qualified Runtime Tasks reach Code TUI"
  />
</p>

|业主|责任|
| ---| ---|
|伞 CLI |命令、注册表信任、ACL 策略、确认、组件编排、产品 UX 和可信运行时/网关组合。 |
|插件管理器 |提供者中立的草稿准入、参与者/范围绑定、两次授予/提供者绑定、策略评估、持久规划/应用证据、精确的提供者重建和确认重播、进程内使用授权转发、功能切换和围栏管理的工作区恢复。 |
| A3S使用|清单验证、依赖解析、不可变生成、提供者/资助计划语义、收据、日志、绑定和功能协调。主机分配和策略不受包控制。 |
|代码生命周期主机 |将包生命周期委托给共享使用托管工厂并使用其驻留类型化功能快照/光标。它通过核心原子会话目录发布经过验证的托管 MCP 服务器、技能、合格的提供者限定的运行时工具任务、摘要绑定的不可查询的知识表面准备情况、依赖关系封闭的本地流程以及有界的无路径 UI 值，并提供特定于生成的租约提供者，该提供者为每个运行或主机句柄获取一个真实的使用快照租约。运行时执行、MCP 传输、A3S 流执行、渲染、范围感知本地 OKF 查询、类型化运行时/网关退休以及精确生成运行时调度保留其现有所有者。 ||代码 TUI |消耗一个实时所需生成和一个主机选择的插件管理器策略；它没有实现第二个包管理器。内置MCP和动态多范围知识搜索工具保持显式兼容性投影；准备情况证据不会选择认知包或授予查询权限。 |
|代码执行和桌面|使用由一个受信任的插件管理器支持的短期原子托管 MCP/技能/运行时任务/UI 投影。观察程序在第一次运行之前停顿，因此其返回的代码生成/摘要、使用快照游标和运行时任务目录摘要无法与以后的切换竞争。 Manager 通过会话拆卸保持活动状态，以准确生成任务分派和可信 HTTP MCP 路由解析。 Desktop 需要此证据；普通 CLI 执行仅将其用于已准备好的安装，并且不执行隐式使用安装。 |

常驻观察者使用类型化的 A3S 使用能力注册表作为其租约
权威。使用 CLI 响应信封保留架构 v1 及其序列化
注册表保留状态、诊断、MCP 服务、非常驻的模式 v2
命令和 OCR 兼容性覆盖。代码验证表单和
拒绝较旧的注册表，而不是混淆传输兼容性
能力兼容性。

代码配置和插件授权有意分开。的
工作区 ACL 可以配置代理，而只有显式操作员配置
或者用户级配置可以授权插件操作。途易和
管理 MCP 端到端地保留了这种区别。

`flow.json`是Code拥有的可视化设计和部署文档，可以绑定
到一个不可变的已安装 Flow 身份。它不是第二个工作流引擎：
`a3s-flow` 仍然负责预检、持久执行、事件历史记录、
并重播。参见【A3S使用组件平台](docs/a3s-use-component-platform.md)
用于生命周期和已安装的 Flow 合同。

## A3S 代码

`a3s code`是一个代理开发者工作区，而不仅仅是一个聊天提示。它保留了
对话、工具执行、批准、工作区更改、内存和
一份语义记录中的验证证据。

|面积 |产品表面|
| ---| ---|
|编码 |流代理循环、工作区工具、有界图像文件/剪贴板输入、保存文件代码智能、有界差异和实时预览。 |
|检索|精确、增量 BM25、符号、语义和混合工作空间搜索。可选的语义索引在会话拥有的内存中异步构建，并且需要显式的嵌入出口授权；不使用矢量数据库服务或持久矢量缓存。 |
|控制|默认、只读计划和非交互式自动模式，具有精确的拨款、可取消的工作和封闭的自动化工具配置文件。 |
|连续性|持久会话、恢复、优先排队后续、上下文搜索、内存、压缩、具有摘要绑定补丁切换的隔离工作树分支、冲突检查倒带以及具有持久完成通知的本地计划报告循环。 |
|研究|证据优先的 DeepResearch 具有有限的获取、引用、质量门控和 Markdown/HTML 报告。 |
|资产|本地代理、MCP、Skill、Flow 和 OKF 创作；安装的流程和托管知识通过不可变的包标识绑定。 |
|型号| ACL 配置的提供商以及帐户拥有的 Claude Code、Codex、Kimi、WorkBuddy 和 A3S OS 路由。 |
|集成 |用于有界编辑器上下文和差异审查的 VS Code/Cursor/Windsurf 命令，以及经过许可的存储库本机 GitHub 操作。 |

日常命令：

```bash
a3s code
a3s code resume
a3s code exec --mode auto "Fix the focused test and verify it"
a3s code exec --mode plan --tool-policy read-only "Review this workspace"
a3s code exec --web-search enabled "Compare the current published guidance"
a3s code exec --mode auto --tool-policy local-workspace --model provider/model "Fix this offline task"
a3s code exec --image before.png,after.png "Compare these screenshots"
a3s code research --web "compare Tokio and async-std"
a3s code sandbox status
a3s code sandbox setup  # Probes the native platform boundary
a3s code schedule enable daily-triage --every 1d
a3s code schedule notifications
a3s code remote diff <execution-id> --organization <organization-id>
a3s top --json
```

每个 `a3s code exec` 运行都会自动将其转录本保存在同一工作空间范围内
`a3s code`、`a3s code resume` 和 `a3s code session` 使用的会话存储。

普通`a3s code exec`对已经准备好的A3S执行只读发现
使用安装。如果找到，代码会自动发布其经过验证的
托管 MCP/技能/运行时任务/UI 生成并停止短命观察程序
在第一次运行之前。如果缺少 Use，该命令将保留不使用路径并
不联系组件发布服务。 A3S Desktop使用保留的
所需的主机模式：它在启动之前协商确切的功能，并且
拒绝成功的结果，除非 `capabilityRuntime` 证明冻结的代码
目录、精确使用快照游标、表面计数和运行时任务目录
消化。一个受信任的流程拥有的插件管理器服务于这两项审查的任务
调度和不透明的 HTTP MCP 路由解析，直到会话关闭。
范围剪切不会启动内置 MCP、兼容性 Knowledge、Flow 或
插件管理器演示文稿投影。

请参阅[代码编辑器和 CI 集成](docs/code-integrations.md) 进行扩展
安装、操作使用、确切的权限配置文件以及故意
封闭的自动化边界。

默认情况下禁用语义工作区检索。仅在以下位置配置它
用户ACL或故意选择的`--config`文件；自动地
发现的工作空间`.a3s/config.acl`可能会禁用继承检索，但
无法启用它或选择端点。嵌入模型是一个单独的
来自 `default_model` 的提供商路由，因此 DeepSeek 聊天路由不会变成
隐式嵌入端点。精确、全局、增量 BM25 和 RRF 是
无模型 CPU 基线。使用可选的 `local-cpu-embedding` 进行构建
功能可以改为接受来自修订版本和 SHA-256 绑定的 ONNX 模型
可信`local_cpu`块。零配置`local_cpu {}`形式询问
A3S 首次使用时可安装锁定的 ~23 MiB MiniLM/ONNX 捆绑包，
在原子提交之前验证每个文件，并离线重用它。这神器
download 从不授权工作区源出口。 `--offline` 和
`A3S_NO_AUTO_INSTALL=1` 禁止首次使用安装丢失；明确的
`artifact_manifest` 仍然可用于自我管理模型捆绑包。
交互式 TUI 启动会公开锁定的描述符，而无需准备
捆绑包：配置、完整工件准入和 ONNX 构建开始
仅在后台语义索引提交真实的第一帧之后
工作。 `a3s code exec` 仍然渴望，因此一次性命令报告缺少本地
在运行其请求的回合之前的工件。
官方发布档案在 Linux x64/ARM64、Windows x64、
和苹果芯片。 Intel macOS 保留了无模型和远程检索，因为固定的 ONNX 运行时不再传送该目标。原生 CI 练习
每个启用的目标上的摘要锁定模型，并且 x64 构建在模型之前失败
当 CPU 缺少 x86-64-v3 基线时加载。局部推理用途
两个输入微批次和一个进程范围的本机作业来限制峰值内存和
取消恢复。仅 RRF 是排名默认值。值得信赖的
ACL 可以显式添加有界的类型化 `deterministic_reranker` 块
候选、特征、指纹和划痕限制；原始模式/算法
选择器和工作区层覆盖在源/提供者之前被拒绝
出口。它还可以精确选择一个类型的 `line`、`fixed_window`，或
`recursive` 分块块。省略保留行分块、递归
分隔符列表和重叠是经过核心验证的，原始/自定义选择器是
被拒绝，并且非文本文件不会被分割或嵌入。 `a3s config show`
报告后端可用性及其非敏感不可用原因，
工件模式/就绪/修订，有界语义就绪超时，
有效的分块，以及没有秘密的版本化重新排序算法。
CLI 为每个主机工作空间配置一次清单支持的块目录；
每会话检索选项不能覆盖它，而每个会话都会保留一个
孤立的短暂向量索引。
TUI 页脚公开异步检索准备情况，无需轮询 Core
在 120 FPS 渲染路径上。 `/status` 将该快照扩展为索引和目录覆盖率、向量内存/修订、嵌入批量效率、第一
就绪延迟和非文本准入计数。统一`search`呼入
`semantic`和`hybrid`模式渲染验证结果、算法、通道、
重新排序和明确的后备证据； `Ctrl+T` 保留这些诊断信息并
完整的结果体。机器可读的 `code exec` 结果显示相同
没有凭证、端点、向量或源文本的有界状态。参见
[工作空间语义检索](docs/cli-reference.md#workspace-semantic-retrieval)，
【真实DeepSeek ACL-主机评测](docs/workspace-retrieval-evaluation.md)，
【本地CPU型号入场指南](docs/local-cpu-workspace-embedding.md)，
以及[跨项目 WSR 路线图](https://github.com/A3S-Lab/Code/blob/main/ROADMAP.md#6-workspace-retrieval-program)。

### 无头特工发布

`a3s code harness` 运行由一个声明的不可变代理释放过程
承认`.a3s/asset.acl`清单。该清单对于 HTTP 具有权威性
端口、就绪和活跃路径、关闭截止日期、协议版本、
能力要求、外部秘密槽位和工件身份；的
唯一的主机覆盖是监听接口。

```bash
a3s code harness --manifest /app/.a3s/asset.acl
a3s code harness --manifest /app/.a3s/asset.acl --listen 127.0.0.1
```

版本一服务公开清单声明的健康路径以及：

|方法与路径|合同|
| ---| ---|
| `POST /v1/agent/commands` |具有不可变运行标识的精确启动、取消和检查点恢复命令。 |
| `POST /v1/agent/events:page` |现有无损`EventEnvelopeV1`流的有界页面。 |
| `POST /v1/agent/changes` |用于一个终端执行的不可变的、经过摘要检查的与 Git 兼容的更改集。 |

清单和兼容性许可、所需的外部秘密检查、
发布前配置加载、Agent初始化完成
端口已绑定。使用 `--json` 或 `--output jsonl`，无效协议和
不支持的功能级别保留其稳定的代理发布错误代码。
运行状况和结构化错误响应不包含秘密值或发布身份。
`SIGINT` 和 `SIGTERM` 在耗尽请求并关闭之前使就绪状态为 false
`health.shutdown_grace_seconds` 内的安全带。

封闭清单模式、存储边界、兼容性规则和
重大变更政策记录在
【A3S代码代理发布合约](https://github.com/A3S-Lab/Code/blob/main/manual/AGENT_RELEASE_CONTRACT.md)。

有用的 TUI 输入：

```text
@src/main.rs                  attach a workspace file
! cargo test -p my-crate      run a direct shell turn
/status                       inspect session, model, modes, and token usage
/ide                          open the workspace browser and editor
/fork worktree               create an isolated branch, workspace, and session
/worktree handoff            emit a SHA-256-bound binary Git patch + manifest
/permissions                  change next-turn mode or review exact grants
/use status                   inspect Use setup and live capabilities
/packages                     review enable/disable for installed cognitive packages
/flow run                     run an exact installed Flow locally
/goal <outcome>               start a durable goal
/loop schedule daily-triage 1d  run an audited L1 report loop in the background
```

按`/`浏览分组命令面板。搜索匹配命令名称，
描述和常见概念，例如`git`或`auth`；拼写错误或
未知的斜杠命令在本地被拒绝并带有建议，而不是被拒绝
发送到模型。

交互式启动在终端切换之前构建一个完整的会话。
新的启动仅创建新的会话 ID； `resume` 仅使用保存的
会话路径并拒绝用空会话替换不可读的历史记录。
显式 `resume <session-id>` 直接探测 id 并枚举其他
仅当必须打印丢失会话诊断时才显示会话。最初的
会话和每个进程内会话重建共享一个惰性文件支持的内存
处理：打开TUI不解码`index.json`；第一次真正的回忆，
写入或检查初始化一次。恢复大于 128 的历史记录
语义条目首先绘制最新的窗口，同时保留每个条目；
Page Up、Ctrl+Home 或鼠标滚轮导航至较旧的输出可以使
完整的成绩单。状态栏分支发现直接读取`.git/HEAD`，
包括链接工作树间接，并且从不启动 Git 子进程。
该命令在构建时立即打印 `Loading workspace…` 指示器
正确性关键会话。它的第一个TUI框架保留了非阻塞
后台服务汇聚时加载线，编辑器已准备就绪
用于输入。 macOS PTY 回归强制执行三秒处理第一帧
天花板。

Evolution内存同步、本机WebView发现或安装、
A3S 使用准备、配置 MCP 传输、本机沙箱初始化
和探测、状态栏和选择器元数据、中断运行
恢复、存储库清单发现和观察者注册以及工作区
将所有等待嵌入一个显式的第一帧刷新确认。之前
清单激活，工作区操作保留其直接本地回退。
清单门是第一个记录的帧后操作，并且在之前打开
共享门释放独立产生的后台服务员。
没有固定的睡眠估计渲染器进度。初始会话已经拥有
失败关闭的沙箱代理，因此早期的标准 Bash 调用会短暂等待
准备就绪，然后返回准备错误；它永远不会落入
未经审查的主机执行。当每个 MCP 工具热插入到活动会话时
服务器已准备就绪，并在模型或工作会话重建后再次进行投影。
Codex 信任根和 TLS 连接器在第一个网络请求上加载，
并且其 OAuth 刷新客户端仅在未经授权的响应后创建。

设置 `A3S_CODE_STARTUP_TRACE=1` 将无内容阶段计时打印到 stderr：

```bash
A3S_CODE_STARTUP_TRACE=1 a3s code
```

跟踪通过 `terminal_handoff` 记录前台阶段，然后是准确的
`first_frame_flushed` 和 `first_deferred_operation` 订购里程碑。它
仅包含静态阶段/操作名称和经过的毫秒数。参见
[启动、会话和安全](docs/cli-reference.md#startup-sessions-and-safety)
用于测量基线和相位定义。

### 本地命令沙箱

TUI 和 `a3s code exec` 共享相同的经过验证的本地进程沙箱。
默认和自动在该边界内运行普通 Bash 调用；计划暴露无
猛击。显式 `require_escalated` 请求永远不会静默转义：默认
要求提供准确的主机命令，而 Auto 拒绝它。如果沙箱准备或
它的本机功能探测失败，Bash 在每种模式下都被拒绝。灾难性的
在任何情况下，命令和凭证或控制路径仍然是硬性拒绝。

沙箱是用 Rust 实现的，仅调用本机操作系统隔离
边界：macOS 上的安全带、Linux 上的用户/PID/网络命名空间以及 seccomp，
以及 AppContainer 以及 Windows 上的终止关闭作业对象。没有 Node.js，
npm 包、sidecar 运行时或一次性提升的设置。 Linux 需要
`bubblewrap` 和可用的非特权用户命名空间； macOS 和 Windows 不需要
额外的沙盒包。 CLI 在使用前探测真实的操作系统边界。
TUI 在终端切换之前附加一个故障关闭代理并将其标记为就绪
仅在帧后探测成功后； `code exec` 保持急切的探测。
这两条路径都不会默默地退回到未沙盒的进程。

对于必须保留完整本地编码能力的无人值守存储库工作
没有公共网络访问权限，`code exec --mode auto --tool-policy
local-workspace` 公开工作区读取、代码智能、有界编辑、
结构化的本地 Git，以及受管理的批处理、程序、任务、工作流程和技能
执行。主机 Web/下载/运行时/知识/托管工具/MCP 入口点和
未知的动态工具保持隐藏和拒绝。 Bash 仅出现在本机沙箱之后
探测成功，无法升级到主机，并使用沙箱的空
网络白名单；委派和技能运行继承相同的边界。的
结构化 Git 工具没有获取、推送、拉取或克隆操作。

沙箱拒绝网络出口和本地侦听器，限制写入
工作区和私有临时目录，保护存储库/控件
元数据，隐藏常见凭证存储和嵌套 `.env*` 文件，清理
周围环境，并拒绝凭证硬链接别名。委托和
技能子运行继承相同的冻结沙箱和权限快照。

### 本地预定循环

经过审核的 L1 工程循环可以通过工作区本地单例工作程序运行：

```bash
a3s code schedule enable daily-triage --every 1d
a3s code schedule run daily-triage
a3s code schedule status
a3s code schedule notifications
a3s code schedule disable daily-triage
```

工作人员自动声明每次到期运行，跳过错过的间隔重放风暴，
记录中断的工作，而不默默地重新运行不确定的效果，并保留
等待通知，直到 TUI 或 CLI 呈现并确认它们。其
内部执行配置文件公开有界工作区读取，`git status`/`log`，
并仅写入所选循环的 `STATE.md`、`RUN_LOG.md` 和 `reports/`
文物。 Shell、网络访问、MCP、运行时、委派、包执行、
未知工具、列入黑名单的路径和活动 ACL 配置保持关闭状态。

发射终端退出后，工作人员仍保持脱离状态。打开途易
当启用的计划存在时重新启动它；操作系统重新启动后，使用
TUI 或 `a3s code schedule start` 恢复当地时间表。

## 组件生命周期

基本安装包含伞式 CLI 和 A3S 代码。可选产品
保持单独发布：

|组件|包含 |公共路线 |生命周期|
| ---| ---| ---| ---|
|代码|是的 | `a3s code` |从伞形可执行文件运行并使用编译到 A3S Code Core 中的本机沙箱。 |
|盒子|没有 | `a3s box`、`a3s compose` |可见的首次使用安装或显式准备。 |
|长凳|没有 | `a3s bench` |显式安装；兼容的公共控制组件版本仍然是一个门槛。 |
|搜索 |没有 | `a3s search` |显式组件安装；嵌入式代码搜索和浏览器引擎保留单独的生命周期。 |
|使用 |没有 | `a3s use`、`a3s code` |显式安装；在政策允许的情况下异步 TUI 首次使用准备；所需的桌面一次性设置；普通 Code Exec 执行仅安装的发现，无需突变。 |
|莫莉|依赖于版本 |默认代码网络搜索无头后端|捆绑每个目标运行时； source/Cargo 安装使用带有跨进程锁定的经过摘要验证的共享缓存。 |
|网页视图 |依赖于版本 |本机 RemoteUI 窗口 |具有浏览器回退功能的托管本机伴侣。 |

```bash
a3s list
a3s info use --versions --sources
a3s install use --source release --dry-run --json
a3s install use --source release --plan-digest <reviewedSha256> --json
a3s upgrade use --yes
a3s doctor use
a3s uninstall use --yes
```

检查下载的版本的目标、清单、摘要、所有权和
活动收据更改之前的健康状况。变异批次使用跨进程
锁定和耐用的检查点；升级中断或失败会留下
上一代健康一代可用。

## 安全和配置

|模式|工作空间 |主机外壳|过境点 |
| ---| ---| ---| ---|
|默认|有限的读取和写入遵循工作区策略。 |普通命令使用经过验证的沙箱；显式主机升级进入审查，并且缺少沙箱执行被拒绝。 |精确的一次性、会话或项目资助。 |
|计划|只读发现。 | bash 不可用。 |批准开始单独的默认回合。 |
|汽车 |受管控操作在没有提示的情况下运行。 |普通命令使用经过验证的沙箱；主机升级或缺少沙箱被拒绝。 |硬性工作空间和政策否认仍然具有权威性。 |

主代码会话仅在托管时接收`use_knowledge_search`
OKF 投影已激活；默认、计划、自动和研究证据收集
将其视为有界只读检索。精确托管的运行时任务显示为
仅在快照生成和命名时使用保守的 `use_tool_*` 工具
已审查的提供商可用。专用的 Use 工作人员仅接收
已验证的包技能、`mcp__use_*`和`use_tool_*`工具。它没有
工作区 shell、不相关的 MCP 访问或递归委派。套餐
突变和开放世界操作返回到父确认流。

配置使用 A3S ACL，而不是 TOML 或 HCL。分辨率检查显式
`A3S_CONFIG_FILE`，工作空间`.a3s/config.acl`，然后`~/.a3s/config.acl`。

```bash
a3s model list
a3s model current
a3s model use codex/gpt-5.6-sol
a3s model use openai/my-model --scope workspace
a3s auth list
a3s auth login os
```

帐户拥有的提供商可以控制其登录状态； A3S不复制
他们的帐户令牌进入`config.acl`、命令输出、日志或浏览器。

## 平台支持

|平台|当前保证|
| ---| ---|
| macOS arm64 / x86_64 |主要代码、组件、Use、Moli、原生WebView发布目标；本地命令隔离使用`sandbox-exec`。 |
| Linux arm64 / x86_64 |主代码、组件、Use、Moli 和无头运行时发布目标；本地命令隔离使用 bubblewrap 和用户命名空间。 |
| WSL |使用 Linux 运行时和文件系统合约。 |
| Windows x86_64 |预览：具有捆绑 Moli、WebView 和 AppContainer/Job 对象本地隔离的本机 A3S 代码，无需单独的设置步骤。完整的浏览器、六面和故障注入奇偶校验仍然是一个门。 |

## 发布准备

公共0.14.0 CLI可用于A3S代码并携带认知包
架构作为**门控预览**，而不是生产包平台。
促销仍需要满足以下所有条件：

- 发布并在操作上验证官方注册中心信任根；
- 完整的跨平台崩溃注入和真实注册表多根共享
  依赖生命周期覆盖；
- 运行完整的真实流程跨平台审核启用和观察程序
  CLI 和 TUI 的收敛矩阵；
- 完成托管OKF回滚、协调恢复和权限恢复，
  备份轮换、分布式放置；精确发布生成查询
  租赁、范围配额、有界保留、逻辑删除 GC、完整性审计、
  派生索引修复和可验证范围本地备份在
  组成的 SQLite 后端；
- 针对发布支持的 OCI 的完整真实提供商生产资格
  任务、长期工具服务和 HTTP MCP。可信的 Linux ACL 可以
  明确为这三者组成共享 Box 提供程序和私有网关；
  默认值保持未分配并且没有后备。的
  注入的 OCI 任务路径现在证明安装以及离线、重新启动安全审查
  禁用/重新启用、提供商漂移恢复和精确收据支持的任务
  Manager 重新启动后调用。已接受的电话现在租赁其已发布的
  通过输出捕获和运行时清理生成，同时隐藏拒绝
  新来电。 snapshot-v2 观察程序现在将精确的任务投影到 TUI 中
  他们审查的提供商在场。私有网关回归证明
  审查路径运行状况、标准 MCP 初始化、持久路由重启、入场排水，以及准确的收据拥有的清除；使用合同测试证明
  用于卸载和上一代清理的服务停用。 Linux
  真实进程门还证明保留的 N/N+1 工具和 MCP 路由，
  主机重启、独立工具和MCP提供程序进程丢失、
  同代端点重新绑定、兄弟隔离、排出、精确移除、
  并且没有残留的运行时或 PID 状态。非 Linux 提供商的组成和
  跨平台恢复矩阵保持开放；
- 关闭剩余的本机 Windows 六面和故障注入
  包生命周期对等；和
- 完成审查的活动渲染和后端绑定
  CLI、TUI 和本机中的世代感知就绪性和沙箱组合
  主机；和
- 定义超出流程的生产计划、恢复和保留
  当前单节点本地运行时。

在这些大门关闭之前，不可用的功能仍然可见
不可用且关闭失败。

## 发展

直接在此存储库中工作或通过 A3S monorepo 的固定
`crates/cli`子模块。不要在 monorepo 根目录下创建 Rust 工作区。

锁定文件将发布的包固定到确切的版本和可组合代码，
Flow、Memory 和 Search 是不可变 git 修订版的伴侣。发布
预检会在发布前验证每个版本和每个平台 Moli 存档
存档或 crate 已发布。

```bash
cargo fmt --all -- --check
cargo test --lib
cargo test --tests
cargo clippy --all-targets --all-features -- -D warnings
```

重点包主机门：

```bash
cargo test --lib use_registry::tests:: --no-fail-fast
cargo test --bin a3s tui::panels::packages::tests --no-fail-fast
cargo test \
  generation_watch_hot_plugs_skill_mcp_runtime_task_flow_and_knowledge_across_tui_replacement
cargo test --lib \
  active_run_pins_native_use_skill_generation_across_atomic_cutover
cargo test --lib \
  use_registry::runtime_tasks::tests --no-fail-fast
cargo test --bin a3s \
  scoped_agent_discovers_and_invokes_only_the_reviewed_runtime_task
cargo test --lib \
  use_registry::knowledge::tests --no-fail-fast
cargo test --bin a3s \
  bound_flow_deploy_resolves_fake_use_catalog_before_os_mutation
cargo test --lib \
  code_host_preflights_flow_and_persists_exact_generation_binding
cargo test --lib \
  signed_okf_install_upgrade_restart_query_and_uninstall_use_code_host
cargo test --lib \
  reviewed_managed_runtime_graph_rejects_drift_and_persists_exact_grant
cargo test --lib \
  signed_workspace_install_is_exact_fenced_and_replayable_after_restart
cargo test --lib \
  reviewed_managed_runtime_graph_rejects_drift_and_persists_exact_grant
cargo test --lib plugin_manager::operation
cargo test --lib plugin_manager::managed_host
cargo test --lib components::cognitive_lifecycle
```

真正的独立进程使用集成是从 monorepo 编排的，因此
其货物输出保持隔离：

```bash
just use-hotplug-e2e
```

## 文档

- [CLI参考](docs/cli-reference.md)
- [CLI产品设计](docs/cli-product-design.md)
- 【CLI技术架构](docs/cli-technical-architecture.md)
- [A3S使用组件平台](docs/a3s-use-component-platform.md)
- [插件授权政策](docs/plugin-authorization-policy.md)
- [代码智能](docs/code-intelligence.md)
- [工作空间检索ACL-主机评估](docs/workspace-retrieval-evaluation.md)
- [DeepResearch证据优先设计](docs/deep-research-evidence-first-redesign.md)
- [不可变代理发布合约](https://github.com/A3S-Lab/Code/blob/main/manual/AGENT_RELEASE_CONTRACT.md)
- [A3S使用网站](https://a3s-lab.github.io/Use/)
- [A3S使用套餐合约](https://github.com/A3S-Lab/Use/tree/main/docs)

## 更新中

```bash
a3s self update --check
a3s self update
a3s upgrade use
```

`a3s update` 和 `a3s update <component>` 仍然是兼容性别名，但
已弃用。 TUI `/update` 保存当前会话、更新 CLI 并
恢复它。组件升级保留了它们自己的来源。

## 执照

A3S CLI 根据 [MIT 许可证](LICENSE) 获得许可。发布档案保留
其捆绑组件的许可证和出处通知。