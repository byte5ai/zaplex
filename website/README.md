# Zap Website

The production entry points direct users to the current [Zaplex README](https://github.com/byte5ai/zaplex#install),
[user guide](https://github.com/byte5ai/zaplex/blob/main/docs/release/1.0-user-guide.md), and
[release artifacts](https://github.com/byte5ai/zaplex/releases). The HTML files under `design/` are
historical upstream visual references; their product claims and build commands are not current installation instructions.
No publication or deployment is implied by this source tree.

Astro 站点,源自 `design/` 下的视觉稿。

```bash
npm install
npm run dev      # http://localhost:4321
npm run build    # 输出 dist/
```

结构:

- `src/pages/index.astro` — Landing
- `src/pages/docs/[...slug].astro` — 文档动态路由
- `src/content/docs/*.mdx` — 文档内容(Content Collections)
- `src/components/` — Nav / Footer / Banner 等
- `src/styles/` — 设计 token 与全局样式
