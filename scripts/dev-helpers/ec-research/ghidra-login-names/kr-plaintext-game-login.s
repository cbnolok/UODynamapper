# Audited UOKR.exe only. Assemble with GNU as --32, link at 0x5b6c8d,
# then objcopy -O binary -j .text. Embedded by client/kr_game_login.rs.
# Replaces the E3 callback registration, reusing the original function's frame
# and shared-pointer cleanup at 0x5b6d69 (EDI: Huffman; EBX: seed message).
.intel_syntax noprefix
.code32
.global _start
.text
_start:
    push 0x20
    call 0x5f94d0                 # operator new, as in the E3 handler
    add esp, 4
    test eax, eax
    jz allocation_failed
    mov esi, eax
    call 0x5dd850                 # Huffman constructor (object in ESI)
    mov esi, eax
    push esi
    lea eax, [esp + 0x18]        # local shared-pointer control block
    push eax
    call 0x5bca20                 # shared_ptr<UOHuffmanCoding_S2C_Decompress>
    mov edi, [esp + 0x14]
    lock inc DWORD PTR [edi + 4] # by-value argument, consumed by AddReceiveTransform
    push edi
    push esi
    push DWORD PTR ds:0xc9f918  # borrowed game connection; global owns its lifetime
    call 0x5a29e0                 # AddReceiveTransform(connection, shared_ptr)
    mov eax, [esp + 0x24]        # original LoginManager argument
    call 0x5b59d0                 # send normal 0x91; no cipher installed
    jmp 0x5b6d69                  # release local Huffman reference, then seed message
allocation_failed:
    push 1
    call 0x5b3b90                 # Disconnect(1)
    jmp 0x5b6d97                  # release only the seed message
