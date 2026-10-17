# Thistle Tea's code-built creature scripts as JSON, for
# src/game/data/tt_scripts.json.gz (src/game/data/tt_scripts.rs reads it). Run
# in Thistle Tea's tree, compiled:
#
#   mix run --no-start --no-compile tools/dump_tt_scripts.exs > tt_scripts.json
#   gzip -9 -n -c tt_scripts.json > src/game/data/tt_scripts.json.gz
#
# Atoms are written ":name", tuples {"t": [...]}, structs as maps with a
# "__struct__" key naming the module's last part (its fields at their defaults
# left out), and map keys as strings.

alias ThistleTea.Game.Core.AI.CreatureScript

defmodule TTDump do
  def enc(nil), do: nil
  def enc(true), do: true
  def enc(false), do: false
  def enc(a) when is_atom(a), do: ":" <> Atom.to_string(a)
  def enc(n) when is_number(n), do: n
  def enc(b) when is_binary(b), do: b
  def enc(f) when is_function(f), do: ":fn"
  def enc(t) when is_tuple(t), do: %{"t" => t |> Tuple.to_list() |> Enum.map(&enc/1)}
  def enc(l) when is_list(l), do: Enum.map(l, &enc/1)
  def enc(%MapSet{} = s), do: s |> MapSet.to_list() |> enc()

  # A struct's fields at their defaults are left out.
  def enc(%mod{} = s) do
    defaults = mod.__struct__() |> Map.from_struct()

    s
    |> Map.from_struct()
    |> Enum.reject(fn {k, v} -> Map.fetch(defaults, k) == {:ok, v} end)
    |> Map.new()
    |> enc()
    |> Map.put("__struct__", mod |> Module.split() |> List.last())
  end

  def enc(m) when is_map(m), do: Map.new(m, fn {k, v} -> {key(k), enc(v)} end)

  defp key(k) when is_atom(k), do: Atom.to_string(k)
  defp key(k) when is_binary(k), do: k
  defp key(k), do: inspect(k)
end

entries = CreatureScript.entries() |> Enum.uniq() |> Enum.sort()

out = %{
  "entries" => entries,
  "events" => Map.new(entries, &{Integer.to_string(&1), TTDump.enc(CreatureScript.events(&1))}),
  "routes" => TTDump.enc(CreatureScript.routes()),
  "gossip" => TTDump.enc(CreatureScript.gossip()),
  "quest_start_steps" => TTDump.enc(CreatureScript.quest_start_steps()),
  "quest_end_steps" => TTDump.enc(CreatureScript.quest_end_steps()),
  "summon_entries" => CreatureScript.summon_entries() |> Enum.uniq() |> Enum.sort()
}

IO.puts(Jason.encode!(out))
