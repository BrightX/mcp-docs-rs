# 项目文档索引

本目录集中存放 mcp-docs-rs 的需求、设计、进度与开发规范文档。**所有正式项目文档一律放在 `docs/` 下**。

| 文档 | 内容 |
|---|---|
| [requirements.md](requirements.md) | 需求：背景、目标、用户场景、功能/非功能需求、约束与非目标 |
| [design.md](design.md) | 设计：工程结构、数据模型、解析算法、存储布局、缓存、MCP 接口、测试策略、风险 |
| [roadmap.md](roadmap.md) | 进度计划与里程碑（M0–M9），含各阶段验收标准与状态 |
| [conventions.md](conventions.md) | 开发规范：代码质量、代码风格、注释规范、提交规范、工作流 |
| [lessons.md](lessons.md) | 错题集：分章节记录踩过的坑，编号 `#x.y` |
| [issues.md](issues.md) | 缺陷跟踪：功能性缺陷（区别于 lessons 的认知踩坑），编号 `E-x.y` |
| [glossary.md](glossary.md) | 术语与代码位置对照：分章节记录术语 / 别名 → 代码位置 |

## 约定

- 文档语言：简体中文；技术术语、代码标识符、路径保持原样。
- 文档随代码演进同步更新，不做一次性产物。
- 开发规范（代码质量 / 风格 / 注释 / 提交）见 [conventions.md](conventions.md)。
- 文档变大时按 [conventions.md](conventions.md) §6「文档管理」拆为目录并归档历史，不做无限追加。
- 事实以实测为准：涉及 rustdoc 输出结构的描述，均在真实产物上核对过（见 design.md「实测事实底座」）。
