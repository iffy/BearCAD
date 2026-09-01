---
sidebar_position: 6
title: Zoom loupe
---

import useBaseUrl from '@docusaurus/useBaseUrl';

# <img src={useBaseUrl("/img/icons/zoom_loupe.svg")} width="30" /> Zoom loupe

Rings a detail on a projection and redraws it larger elsewhere on the page — the
detail-view callout of a printed drawing.

Four clicks, each circle drawn from its centre like the **Circle** tool: click the detail
on a projection, move out and click to size the ring; then click where the big circle
goes and move out and click to size it. **Esc** cancels a half-made one.

The magnification is the ratio of the two circles, so growing the big one zooms in
further rather than showing more. Both circles and the line joining their rims stroke
thinner than the model outline.

With **Select**, click either circle — or the loupe's row in the Elements pane — to select
it. Drag it by the middle to move it, or by its outer ring to resize. **Delete** on either
drops the pair.

A selected loupe has its own **Style** in the Context pane, independent of the projection.
**Same as the view** follows the card; any other style draws only the magnified detail that
way. Right-click the loupe for the same list.

The **Dimension** tool can dimension a magnified edge. The label is the edge's real
length. If the edge isn't fully inside the ring, that end of the dimension line
finishes in dashes instead of an arrow — the number is the whole edge, not the
cropped bit.

The Lua API is under [Scripting](/docs/scripting/drawings#zoom-loupes).
