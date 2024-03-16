# CNLS

Class Name Language Server.

Features:

- [x] Hover
- [x] Go To Definition
- [] Completions

## Demo

## Install

## Usage (nvim)

```lua
local nvim_lsp = require("lspconfig");
local configs = require 'lspconfig.configs'

-- cnls setup
if not configs.cnls then
  configs.cnls = {
    default_config = {
      cmd = { "cnls" },
      filetypes = { "javascript", "javascriptreact", "javascript.jsx", "typescript", "typescriptreact", "typescript.tsx" }
    },
  }
end

nvim_lsp.cnls.setup({
  root_dir = nvim_lsp.util.root_pattern("package.json"),
  capabilities = lsp_capabilities,
  settings = {
    cnls = {
      scopes = { "att:className,class", "fn:createElement" }
    }
  }
})
```

### About Scopes

You may have tailwind classes in other places besides `className="..."`, or even `cva(...)`.
For examples, the `classes` prop in mui components.

You can define places for `cnls` to look for classes with the `cnls.scopes` settings.
The syntax for a scope is <variant>:<...values>

**Variants** are:

- `fn` to target a function call (e.g 'fn:cva')
- `att` to target a jsx attribute (e.g. 'att:className')
- `prop` to target a jsx attribute (e.g. 'prop:className')

**Values** are strings, and you can use a wildcard `*` at the begining or the end.
For example 'att:className att:\*ClassName' will find classes all of these attributes

```js
<Btn
  className="w-10 bg-red"
  iconClassName="text-black"
  textClassName="text-xl"
/>
```

By default `cnls` uses 'att:className,class fn:createElement'.
