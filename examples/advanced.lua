-- Advanced Fenestre config (Lua).
--
-- The Lua loader evaluates this script and reads the returned table, so you can
-- use variables, helper functions, and loops to avoid repeating yourself. This
-- file is intentionally small but shows the patterns TOML cannot express.

-- Reusable modifier sets (DRY instead of repeating "super" everywhere).
local super = { "super" }
local supShift = { "super", "shift" }
local supAlt = { "super", "alt" }
local supAltShift = { "super", "alt", "shift" }

-- Helper to build a keybinding table.
local function kb(keysym, mods, cmd)
	return { keysym = keysym, modifiers = mods, command = cmd }
end

-- Helper to build a spawn command.
local function spawn(prog, ...)
	local cmd = { "spawn", prog }
	for _, arg in ipairs({ ... }) do
		cmd[#cmd + 1] = arg
	end
	return cmd
end

-- One map drives focus / move / resize bindings via a single loop.
local dirs = { left = "h", down = "j", up = "k", right = "l" }

local keybindings = {
	kb("Return", super, spawn("foot")),
	kb("q", super, "close"),
}

for dir, key in pairs(dirs) do
	keybindings[#keybindings + 1] = kb(key, super, "focus_" .. dir)
	keybindings[#keybindings + 1] = kb(key, supShift, "move_" .. dir)
	keybindings[#keybindings + 1] = kb(key, supAlt, "resize_expand_" .. dir)
	keybindings[#keybindings + 1] = kb(key, supAltShift, "resize_shrink_" .. dir)
end

-- Conditional layout: tighten gaps on smaller setups.
local gap = 10
if os.getenv("FENESTRE_SMALL_GAP") then
	gap = 4
end

-- Generate floating rules from a plain list instead of repeating tables.
local floating_apps = { "steam", "org.mozilla.Thunderbird", "libreoffice-" }
local rules = {}
for _, app in ipairs(floating_apps) do
	rules[#rules + 1] = { app_id = { value = app, match = "prefix" }, mode = "floating" }
end

return {
	layout = {
		gap = gap,
		margin_top = 30,
		margin_right = 10,
		margin_bottom = 10,
		margin_left = 10,
	},

	decorations = false,
	border_width = 2,
	border_color_focused = 0xff0000ff,
	border_color_unfocused = 0xff888888,

	keybindings = keybindings,
	rules = rules,
}
