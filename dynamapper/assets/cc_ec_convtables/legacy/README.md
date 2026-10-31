Legacy EC conversion-table material.

Active terrain data now comes from:
- TerrainDefinition.uop, packed into tex_land_ec.uddp provenance.
- EcTerrainOverrides.kdl for narrow reviewed facts not derivable from UOP data.
- TerrainTranscode.kdl as the remaining active CC land id to EC material fallback table.

The *_sm transcode experiments are retained here for reference only.
TerrainDefinition.kdl, the hand-written terrain table from before
TerrainDefinition.uop was parsed, was removed; git history keeps it. The dated subfolder is a snapshot of the active conversion-table
state before terrain blending work started.
