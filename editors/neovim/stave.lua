-- Stave for Neovim. Put this file in ~/.config/nvim/plugin/ (or require it
-- from your init.lua) and put stave-lsp on PATH.
--
-- Neovim 0.11 and later configure language servers with vim.lsp.config;
-- with nvim-lspconfig on an older Neovim, see the second half of this file.
-- Colouring comes from stave-lsp's semantic tokens (there is no tree-sitter
-- grammar for Stave yet); the highlight links at the end give the custom
-- token types and modifiers a look.

vim.filetype.add({ extension = { stave = "stave" } })

vim.api.nvim_create_autocmd("FileType", {
  pattern = "stave",
  callback = function()
    vim.bo.commentstring = "// %s"
    vim.bo.comments = ":///,://"
    vim.bo.shiftwidth = 2
    vim.bo.tabstop = 2
    vim.bo.expandtab = true
  end,
})

if vim.lsp.config ~= nil then
  -- Neovim 0.11+.
  vim.lsp.config("stave", {
    cmd = { "stave-lsp" },
    filetypes = { "stave" },
    root_markers = { ".git" },
  })
  vim.lsp.enable("stave")
else
  -- Older Neovim with nvim-lspconfig.
  local ok, lspconfig = pcall(require, "lspconfig")
  if ok then
    local configs = require("lspconfig.configs")
    if configs.stave == nil then
      configs.stave = {
        default_config = {
          cmd = { "stave-lsp" },
          filetypes = { "stave" },
          root_dir = lspconfig.util.root_pattern(".git"),
          single_file_support = true,
        },
      }
    end
    lspconfig.stave.setup({})
  end
end

-- Semantic token styles: @lsp.type.<type>.stave and @lsp.mod.<modifier>.stave
-- (the legend is editors/semantic-tokens.json).
local function link(group, target)
  vim.api.nvim_set_hl(0, group, { link = target, default = true })
end
link("@lsp.type.docComment.stave", "@comment.documentation")
link("@lsp.type.unit.stave", "@type.builtin")
link("@lsp.type.part.stave", "@type")
link("@lsp.type.assembly.stave", "@type")
link("@lsp.type.joint.stave", "@function.method")
link("@lsp.type.hardware.stave", "@constructor")
link("@lsp.type.param.stave", "@variable.parameter")
link("@lsp.type.role.stave", "@variable.parameter")
link("@lsp.type.let.stave", "@constant")
link("@lsp.type.region.stave", "@variable.member")
link("@lsp.type.stock.stave", "@type.definition")
link("@lsp.type.profile.stave", "@type.definition")
link("@lsp.type.recipe.stave", "@function")
link("@lsp.type.scene.stave", "@label")
link("@lsp.type.require.stave", "@label")
link("@lsp.type.label.stave", "@constant")
link("@lsp.type.member.stave", "@property")
vim.api.nvim_set_hl(0, "@lsp.mod.reference.stave", { underline = true, default = true })
vim.api.nvim_set_hl(0, "@lsp.mod.solvedParam.stave", { bold = true, default = true })
vim.api.nvim_set_hl(0, "@lsp.mod.defaultLibrary.stave", { italic = true, default = true })
vim.api.nvim_set_hl(0, "@lsp.mod.unresolved.stave", { undercurl = true, default = true })
