# Design reference

Visual reference for the Trailscope UI. Match the layout, spacing, typography, colour tokens and component styling shown here.

- `01-live-tail.html`: query bar (Search / Filters / SQL WHERE + quick filters), overview tiles, route heatmap, new & spiking errors, live stream table
- `02-request-inspector.html`: nearby requests, request header + actions, error card, body/response/headers/cURL tabs, details panel, timing waterfall
- `03-journey.html`: journey search, summary tiles, service swimlanes, event log, context cards

These files are design mockups, not production code. Inline styles carry the exact values. The `<script>` block at the bottom of each file holds the sample data and small bits of UI state. Sample data (routes, IDs, error names) is illustrative only. Keep the product domain-neutral as the main prompt says.
