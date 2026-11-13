# 阶段验证产物

- `validation/<阶段>/fixtures/` 保存生成的验证输入。
- `validation/<阶段>/tools/` 保存验证专用的本地依赖。
- `validation/<阶段>/runs/<运行名>/` 保存单次验证的日志、采样和报告。
- `dist/<版本>/` 保存待验收的发布附件与校验值。

生成文件由 Git 忽略。阶段记录在 `docs/stages/` 中维护。根目录的 `target/` 用于日常编译，正式附件从独立的发布构建和装配流程生成。[工程与产物](../docs/G-05-contracts.md#工程与产物)
