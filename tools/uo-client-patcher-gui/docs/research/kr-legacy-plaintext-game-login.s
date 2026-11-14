# KR 2.45.4.4, callback replacement at VA 0x504837, within its original 397-byte function.
# Other exact profiles relocate the audited calls and game singleton operand.
# The original aligned EBP frame, FS:0 registration and seed lifetime remain intact.
# At entry ESP=B: seed shared_ptr at B+0x10, Huffman shared_ptr at B+0x18,
# raw allocation at B+0x0c and exception state at B+0x34. EBX=1.
.intel_syntax noprefix
.code32
.text
.global _start
.equ RAW_ALLOCATION, 0x0c
.equ HUFFMAN_LOCAL, 0x18
.equ EH_STATE, 0x34
.equ HUFFMAN_SIZE, 0x20
.equ RAW_OWNED, 5
.equ SHARED_OWNED, 6
.equ GAME_READY, 2
.equ GAME_STATE, 0x80
.equ GAME_SINGLETON, 0xd09b68
.equ SEED_CLEANUP, 0x5048d0

_start:
    push HUFFMAN_SIZE
    call 0x52e878                 # operator new (cdecl)
    pop ecx
    mov [esp+RAW_ALLOCATION], eax
    mov byte ptr [esp+EH_STATE], RAW_OWNED
    test eax, eax
    jz allocation_failed
    push eax
    call 0x51b1d9                 # Huffman constructor, RET 4
    mov byte ptr [esp+EH_STATE], bl
    lea ecx, [esp+HUFFMAN_LOCAL]
    push ecx
    mov edi, eax
    call 0x50795e                 # typed shared_ptr constructor, object in EDI, RET 4
    mov byte ptr [esp+EH_STATE], SHARED_OWNED

    # Construct the by-value pair immediately before AddReceiveTransform takes ownership of it.
    # The callee destroys this argument on return or exception; state 6 owns the local pair.
    push ecx
    push ecx
    mov eax, esp
    push eax
    lea ecx, [esp+HUFFMAN_LOCAL+12]
    call 0x507999                 # shared_ptr copy, output argument, source in ECX, RET 4
    mov eax, [GAME_SINGLETON]     # retained by the client's existing global shared_ptr
    mov dword ptr [eax+GAME_STATE], GAME_READY
    push eax
    call 0x4f7d4c                 # AddReceiveTransform(game, shared_ptr), RET 12
    mov ecx, [ebp+8]
    call 0x503cca                 # Send91(LoginManager in ECX)

    # The original state-6 unwind action and this normal cleanup dispatch through the control block.
    # They release the typed Huffman shared_ptr despite the old local's callback static type.
    mov byte ptr [esp+EH_STATE], bl
    lea eax, [esp+HUFFMAN_LOCAL]
    push eax
    call 0x45dc93                 # generic shared_ptr release, RET 4
    jmp SEED_CLEANUP

allocation_failed:
    mov ecx, [ebp+8]
    push 1
    call 0x502ac3                 # Disconnect(1), LoginManager in ECX, RET 4
    jmp SEED_CLEANUP              # no Huffman pair was constructed
