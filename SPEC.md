This repository should contain the code for a CAD program named LE3.  LE3 is comparable to AutoDesk Fusion, FreeCAD, OpenSCAD, etc...

# The basics

The program should be easy to compile on macOS, Linux or Windows. It should be written in a compiled language that produces a single, self-contained executable. The executable should launch the GUI by default, but it should also be usable as a command-line tool to perform various operations. 
Evaluate and choose the best language from this list:

1. Nim
2. Zig
3. C
4. Rust

Functionality within the app should be programmable -- if I can do it in the GUI I should be able to do it via programming. I haven't decided if it should be Lua-based or Typescript-based or something else. I don't want a custom DSL.

The saved file format should be SQLite and has the extension `.le3`. You decide the schema, but put something in place so that earlier versions can be upgraded to later versions (i.e. store a list of patches that have been applied within the SQLite file).

It should be able to export to the following file formats: `.3mf`, `.stl`, `.obj`, `.amf`, `.step` (or `.stp`)

It should have great support for units and mixing units. Every component should have default units so that if I enter a number without units it will inherit those. I should be able to specify a length as `3mm + 2in`. It should store that as `3mm + 2in` so that I can see what was entered and update it.

## GUI

- Avoid floating windows and modals in favor of tiled panes.
- There should be default shortcuts for the most common actions
- The user should be able to set a shortcut for every action
- There should be a command palette (like VS Code) that lets you execute context-pertinent commands.
- It should support light mode and dark mode (or maybe just themes)
- You should be able to interact with a 3D rendering (and move it around)

## Hierarchy and linearity

AutoDesk Fusion has a timeline feature that lets you roll back in time. I like the ability to undo things and go back in time, but I don't like the linearity of it.

- LE3 should allow for infinite undoing (even after closing and reopening a file).
- It should show a directed graph of changes/actions rather than a linear timeline. For example, if I have two independent components in a document, they should each have their own graph of changes/actions. If a third component depends on the first two, then the third component's graph should show that dependency on the first two.

## Variables and parameters

Parameters are a first-class feature of LE3. There should be a pane with parameters visible while doing the modeling.
- There should be an option to filter the parameters to only show those relevant to the current task.
- I should be able to use a parameter anywhere a value is accepted
- Every input that takes a value should allow for expressions (e.g. `1 + 2 + lengthOfTheThing / 2`)

## Constraints

LE3 should support the following constraints:

- Coincident - two points are constrained to be at the same point, or a point is constrained to appear somewhere on a line
- Parallel - two lines are constrained to be parallel

## Command Line Tool options

The command-line use of the app should support the following functions:

- To export a le3 file to one of the file formats.
