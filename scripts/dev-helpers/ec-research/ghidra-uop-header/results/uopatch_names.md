# UOP header names

Program: `uopatch.exe`, SHA-256 `7c4ec66d9c53a8e45e35b4eba8fcfd5a7ce96dbf4f3baaea4ae32823fcf44dad`.

Mode: `--apply` (changes: none).

| Address | Name | Checks | Result |
|---|---|---|---|
| `0x0043e3e0` | `UOPatch::RegisterKey20` | const:91, const:14, calls:00458dc0, calls:00407930 | already applied |
| `0x00458dc0` | `UOPatch::ImportPublicKey` | calls:00459250 | already applied |
| `0x00459250` | `UOPatch::rsa_import` | calls:0045ec80, calls:0045b240 | already applied |
| `0x00459050` | `UOPatch::rsa_exptmod` | calls:0045b240, calledby:00458e00 | already applied |
| `0x00458e00` | `UOPatch::rsa_public_decrypt_chunks` | calls:00459050, calls:004599a0 | already applied |
| `0x00458fe0` | `UOPatch::rsa_verify_decrypt` | calls:00458e00 | already applied |
| `0x00459690` | `UOPatch::rsa_signdepad` | calledby:004599a0 | already applied |
| `0x0045b240` | `UOPatch::mp_read_unsigned_bin` | calledby:00459050 | already applied |
| `0x00458b40` | `UOPatch::sha256_memory` | calls:00458900, calls:004589d0 | already applied |
| `0x0042aba0` | `UOPatch::VerifySignature` | calls:00458b40, calls:0043e770, calls:00458fe0 | already applied |
| `0x0042acf0` | `UOPatch::VerifyFileSignature` | calls:0042aba0 | already applied |
| `0x0043e770` | `UOPatch::GetKeyStore` | calledby:0042aba0 | already applied |
| `0x00407930` | `UOPatch::KeyStore_Add` | calledby:0043e3e0 | already applied |
| `0x00438d20` | `UOPatch::PatchProduct` | calledby:00439ff0 | already applied |
| `0x00437c00` | `UOPatch::DeterminePatchSet` | calls:0042e9e0, calls:00437380 | already applied |
| `0x00437380` | `UOPatch::ExamineLocalPacks` | calls:00436350, calledby:00437c00 | already applied |
| `0x00436350` | `UOPatch::ExaminePack` | calls:0042aba0, calledby:00437380 | already applied |
| `0x0042e9e0` | `UOPatch::ExamineUnpackedFiles` | calls:0042acf0, calledby:00437c00 | already applied |
