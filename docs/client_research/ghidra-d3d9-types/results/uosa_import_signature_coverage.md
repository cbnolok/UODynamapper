# Import signature coverage

Program: UOSA.exe
SHA-256: 5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50
Archives:
- windows_vs12_32.gdt
- d3d9_x86.gdt

| Library | Imports | With signature | Without |
|---|---:|---:|---:|
| ADVAPI32.DLL | 10 | 10 | 0 |
| D3D9.DLL | 1 | 1 | 0 |
| D3DX9_34.DLL | 21 | 21 | 0 |
| DBGHELP.DLL | 11 | 0 | 11 |
| DDRAW.DLL | 1 | 0 | 1 |
| DINPUT8.DLL | 1 | 0 | 1 |
| GDI32.DLL | 16 | 16 | 0 |
| KERNEL32.DLL | 187 | 187 | 0 |
| MSS32.DLL | 58 | 0 | 58 |
| SHELL32.DLL | 1 | 1 | 0 |
| SHFOLDER.DLL | 1 | 0 | 1 |
| USER32.DLL | 59 | 59 | 0 |
| VERSION.DLL | 3 | 3 | 0 |
| WINMM.DLL | 4 | 4 | 0 |
| WS2_32.DLL | 24 | 21 | 3 |

## DBGHELP.DLL without signature

- `StackWalk`
- `SymCleanup`
- `SymEnumSymbols`
- `SymFromAddr`
- `SymFunctionTableAccess`
- `SymGetLineFromAddr`
- `SymGetModuleBase`
- `SymGetTypeInfo`
- `SymInitialize`
- `SymSetContext`
- `SymSetOptions`

## DDRAW.DLL without signature

- `DirectDrawCreateEx`

## DINPUT8.DLL without signature

- `DirectInput8Create`

## MSS32.DLL without signature

- `_AIL_3D_rolloff_factor@4`
- `_AIL_allocate_sample_handle@4`
- `_AIL_close_digital_driver@4`
- `_AIL_close_filter@4`
- `_AIL_close_stream@4`
- `_AIL_digital_CPU_percent@4`
- `_AIL_digital_configuration@16`
- `_AIL_digital_master_volume_level@4`
- `_AIL_end_sample@4`
- `_AIL_filter_property@20`
- `_AIL_find_filter@8`
- `_AIL_last_error@0`
- `_AIL_open_digital_driver@16`
- `_AIL_open_filter@8`
- `_AIL_open_stream@12`
- `_AIL_register_EOB_callback@8`
- `_AIL_register_stream_callback@8`
- `_AIL_release_sample_handle@4`
- `_AIL_room_type@4`
- `_AIL_sample_ms_position@12`
- `_AIL_sample_occlusion@4`
- `_AIL_sample_playback_rate@4`
- `_AIL_sample_stage_property@24`
- `_AIL_sample_status@4`
- `_AIL_sample_volume_levels@12`
- `_AIL_serve@0`
- `_AIL_set_3D_rolloff_factor@8`
- `_AIL_set_digital_master_reverb@16`
- `_AIL_set_digital_master_reverb_levels@12`
- `_AIL_set_digital_master_volume_level@8`
- `_AIL_set_file_callbacks@20`
- `_AIL_set_listener_3D_orientation@28`
- `_AIL_set_listener_3D_position@16`
- `_AIL_set_named_sample_file@20`
- `_AIL_set_redist_directory@4`
- `_AIL_set_room_type@8`
- `_AIL_set_sample_3D_distances@16`
- `_AIL_set_sample_3D_orientation@28`
- `_AIL_set_sample_3D_position@16`
- `_AIL_set_sample_loop_count@8`
- `_AIL_set_sample_ms_position@8`
- `_AIL_set_sample_occlusion@8`
- `_AIL_set_sample_playback_rate@8`
- `_AIL_set_sample_processor@12`
- `_AIL_set_sample_reverb_levels@12`
- `_AIL_set_sample_volume_levels@12`
- `_AIL_set_sample_volume_pan@12`
- `_AIL_set_speaker_configuration@16`
- `_AIL_set_stream_loop_count@8`
- `_AIL_set_stream_ms_position@8`
- `_AIL_shutdown@0`
- `_AIL_speaker_configuration@20`
- `_AIL_start_sample@4`
- `_AIL_start_stream@4`
- `_AIL_startup@0`
- `_AIL_stream_ms_position@12`
- `_AIL_stream_sample_handle@4`
- `_AIL_stream_status@4`

## SHFOLDER.DLL without signature

- `SHGetFolderPathA`

## WS2_32.DLL without signature

- `WSAConnect`
- `WSASend`
- `WSASocketA`
