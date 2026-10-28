// Name the UO Enhanced Client login and network code (LoginManager, packet handlers, ciphers,
// packet framing), label its globals and comment the ECLO patch sites; with --apply, rename
// DEFAULT functions, add tags, plate comments, labels and EOL comments.
// @category ECResearch
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.StringDataInstance;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.CodeUnit;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.DataIterator;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionTag;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.scalar.Scalar;
import ghidra.program.model.symbol.Namespace;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.SourceType;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;
import ghidra.util.exception.DuplicateNameException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

public class NameLoginCode extends GhidraScript {
    private static final String APPLY_FLAG = "--apply";
    private static final String TAG_LOGIN = "UO_LOGIN";
    private static final String TAG_NET = "UO_NET";
    private static final String COMMENT_PREFIX = "NameLoginCode: ";
    private static final String PROBABLE = "Probable. ";
    private static final String SCOPE = "::";
    // Executable the expected addresses were taken from (UOSA.exe 4.0.102.3); other builds
    // use the locators only.
    private static final String TESTED_SHA256 = "5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50";

    private static final int POINTER_SIZE = 4;
    private static final int BYTE_MASK = 0xff;
    private static final int WORD_MASK = 0xffff;
    private static final long DWORD_MASK = 0xffffffffL;
    private static final int HEX_RADIX = 16;
    // Packet handler registration: UOClassCallback<shared_ptr<UONetMessageReceive>, LoginManager>
    // object { vftable, this, handler } built at +0, +4, +8, then PUSH id; LEA ECX,[conn+0x24];
    // CALL PacketHandlerMap::Register.
    private static final String CALLBACK_PREFIX = "UOClassCallback<";
    private static final String CALLBACK_OWNER = ",LoginManager>";
    private static final String COUNTED_PREFIX = "sp_counted";
    private static final String VFTABLE = "vftable";
    private static final long CALLBACK_HANDLER_OFFSET = 8;
    private static final long CONNECTION_HANDLER_MAP_OFFSET = 0x24;
    private static final int REGISTRATION_WINDOW = 60;
    private static final int CONNECTION_LOOKBACK = 24;
    // A shared_ptr global is { object, control block }.
    private static final long SHARED_PTR_CONTROL_OFFSET = POINTER_SIZE;
    // Packet registry: the table builder calls FindOrInsert once per packet id.
    private static final int MIN_REGISTRY_ENTRIES = 200;
    // Packet registry entry stack layout: id byte, dynamic flag dword at +1, length dword at +5.
    private static final long ENTRY_FLAG_OFFSET = 1;
    private static final long ENTRY_LENGTH_OFFSET = 5;
    private static final int PACKET_IDS = 256;
    // ECLO patterns (patcher_address.cpp, patcher_encryption.cpp); -1 is a wildcard byte.
    private static final int ANY = -1;
    private static final int[] PORT1_PATTERN = {0xC7, 0x44, 0x24, 0x14, ANY, ANY, ANY, ANY, 0x66, 0xC7, 0x44, 0x24, 0x18};
    private static final int PORT1_IP_IMM = 4;
    private static final int PORT1_WRITE = 8;
    private static final int[] PORT2_PATTERN = {0x89, ANY, 0x24, 0x14, 0x66, 0xC7, 0x44, 0x24, 0x18};
    private static final int PORT2_WRITE = 4;
    // MOV word ptr [ESP+0x18], imm16 is 7 bytes; the imm16 is its last 2 bytes.
    private static final int PORT_WRITE_LENGTH = 7;
    private static final int PORT_IMM_OFFSET = 5;
    // TOL clients: JNZ +7 then the third port write.
    private static final int[] PORT3_PREFIX = {0x75, 0x07, 0x66, 0xC7, 0x44, 0x24, 0x18};
    private static final int JNZ_SHORT_LENGTH = 2;
    private static final String TITLE_FORMAT = "UOSA - %s ";
    private static final int[] TITLE_GUARD = {0x0A, 0x00};
    private static final int TITLE_TRAILER = 2;
    private static final int IP_SLOT_SIZE = 16;
    private static final int IP_SLOTS = 2;
    private static final int FIRST_IP_SLOT = 1;
    private static final int[] ENC1_PATTERN = {0x30, 0x45, 0x00, 0x83, 0x83, 0x4C, 0x12, 0x00, 0x00, 0x01, 0x5D, 0x5B};
    private static final int[] ENC2_PATTERN = {0x30, 0x10, 0x83, 0x46, 0x18, 0x01, 0x8B, 0x45, 0x0C, 0x83, 0xC7, 0x01, 0x3B, 0xFB, 0x72, 0xD5};
    private static final int[] ENC3_PATTERN = {0x30, 0x08, 0x8B, 0x4E, 0x04, 0x8B, 0x46, 0x08, 0x8B, 0xD0, 0xD1, 0xE8, 0x8B, 0xF9};

    private enum Kind { FUNCTION, ANCHOR, LABEL }

    private static final class Spec {
        String key;
        Kind kind;
        String path;
        String tag;
        String locator;
        long expected;
        String evidence;
        boolean probable;
    }

    private static final class Registration {
        Address site;
        Function owner;
        int id;
        Address handler;
        Address connection;
        Address register;
    }

    private final List<Spec> specs = new ArrayList<>();
    private final Map<String, Address> located = new HashMap<>();
    private final Map<String, String> how = new HashMap<>();
    private final List<Registration> registrations = new ArrayList<>();
    private final List<String> anomalies = new ArrayList<>();
    private final List<String> kept = new ArrayList<>();
    private final List<String> ecloRows = new ArrayList<>();
    private final Map<String, Integer> changes = new LinkedHashMap<>();
    private final Map<Integer, String> packetTable = new TreeMap<>();
    private final Set<Integer> reregistered = new LinkedHashSet<>();
    private List<Data> strings;
    private boolean testedBuild;

    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1 || args.length > 2 || (args.length == 2 && !args[1].equals(APPLY_FLAG))) {
            throw new IllegalArgumentException("Expected OUTPUT_FILE [" + APPLY_FLAG + "]");
        }
        boolean apply = args.length == 2;
        // Created before any change, so an existing output file stops the run unchanged.
        try (PrintWriter output = new PrintWriter(Files.newBufferedWriter(Path.of(args[0]),
                StandardCharsets.UTF_8, StandardOpenOption.CREATE_NEW))) {
            analyze(output, apply);
        }
        println("Wrote " + args[0] + (apply ? " (applied: " + changes + ")" : ""));
    }

    // ---- Specifications ----

    // Locators, resolved in order (a locator may only use keys defined above it):
    //   str:PREFIX                 the one function referring to the one string starting with PREFIX
    //   call:KEY:N                 target of the N-th direct call (address order) in KEY
    //   handler:ID                 LoginManager handler registered for packet ID
    //   regsite:ID                 function registering the handler for packet ID
    //   register:ID                PacketHandlerMap::Register called at that registration
    //   conn:ID                    connection global whose handler map receives packet ID
    //   vtable:CLASS:SLOT          function in slot SLOT of CLASS::vftable
    //   read:KEY:N / write:KEY:N   N-th absolute non-code data read/write in KEY
    //   offset:KEY:DELTA           KEY + DELTA
    //   caller:KEY                 the only function calling KEY
    //   othercaller:KEY:EXCLUDED   the only function other than EXCLUDED calling KEY
    //   import:KEY:NAME            the only caller of KEY that also calls the import NAME
    //   registry:FIND              the packet registry insert (caller of FIND called by a table builder)
    //   busiest:KEY                the function with at least MIN_REGISTRY_ENTRIES calls to KEY
    //   eclo:ip1 / eclo:ip2        login host string slots after the "UOSA - %s " title format
    private void defineSpecs() {
        anchor("luaTryLogin", "str:LuaTryLogin: logging in account", 0x005296e0L);
        anchor("luaSelectShard", "str:LuaSelectShard: requesting shard", 0x00529ba0L);
        anchor("luaSelectCharacter", "str:LuaSelectCharacter: requesting c", 0x00529df0L);
        anchor("luaDeleteCharacter", "str:LuaDeleteCharacter: Deleteing ch", 0x0052a050L);

        // Login flow (LoginManager; names from its own log strings where present).
        login("loginAccount", "LoginManager::LoginAccount", "str:LoginManager::LoginAccount: UI Login started",
            0x00613510L, false, "Log \"LoginManager::LoginAccount: UI Login started, account\"; called by "
            + "UOLua::TryLogin. Sets the window title \"UOSA - %s \" and picks the login address: LoginConfig "
            + "+0x9c/+0xa0 when present; else the internal test address when app +0x1c9 is set; else one of "
            + "two hardcoded hosts (resolved) and port 7775 or 7776, each picked by rand() parity (the ECLO "
            + "patch sites). Then calls ConnectToLoginServer.");
        login("setCredentials", "LoginManager::SetCredentials", "call:loginAccount:2", 0x00610280L, true,
            "Copies the account and password strings into LoginManager +0x0/+0x1c; called by LoginAccount "
            + "and AutoLoginFromConfig.");
        login("autoLogin", "LoginManager::AutoLoginFromConfig", "othercaller:setCredentials:loginAccount",
            0x00613710L, true, "Uses the LoginConfig credential (+0x7c) and server (+0x9c/+0xa0) when no "
            + "login is in progress, then calls ConnectToLoginServer.");
        login("connectLogin", "LoginManager::ConnectToLoginServer", "call:loginAccount:14", 0x00610330L, false,
            "Connects the login connection (vtable +0x34), sends the 0xEF seed and installs the login cipher "
            + "seeded with the local IPv4, then builds and sends 0x80 (account, password; 30 bytes each).");
        net("localIp", "UONet::GetLocalIPv4", "call:connectLogin:2", 0x00646880L, false,
            "gethostname, gethostbyname, inet_ntoa, inet_addr, ntohl: the first local IPv4 address in host "
            + "order. Used as the login seed and in 0x91/0x5D.");
        net("resolveHost", "UONet::ResolveHostIPv4", "call:loginAccount:9", 0x00646920L, false,
            "gethostbyname then ntohl: resolves a host name or dotted IP to a host-order IPv4.");
        net("sendSeed", "UOLogin::SendSeedAndInstallLoginCipher", "call:connectLogin:4", 0x00600ae0L, false,
            "Builds packet 0xEF (21 bytes: seed, version 67.0.102.0 hardcoded as 0x43/0/0x66/0), sends it "
            + "unencrypted (connection vtable +0x40), then creates UOLoginCipher_C2S_Encrypt, seeds it and "
            + "adds it as a send transform.");
        net("netMessageSend", "UONetMessageSend::UONetMessageSend", "call:sendSeed:1", 0x00601700L, false,
            "Constructor (buffer, packet id, flags): stores UONetMessageSend::vftable, looks up the packet "
            + "length in the packet registry, writes the id byte and, for variable packets, a big-endian "
            + "length word.");
        net("appendBytes", "UONet::NetMessage_AppendBytes", "call:netMessageSend:2", 0x00646630L, true,
            "Appends N bytes to the outgoing message buffer; used for every packet field.");
        net("writeSeed", "UOPacket::Write_0xEF_Seed", "call:sendSeed:4", 0x006008f0L, false,
            "Appends five dwords (seed, major, minor, revision, prototype), htonl'd when big-endian.");
        net("loginCipherInit", "UOLoginCipher_C2S_Encrypt::Init", "call:sendSeed:8", 0x0063c720L, false,
            "Classic UO login key table from the seed: k0 = ((~s ^ 0x1357) << 16) | ((s ^ 0xffffaaaa) & "
            + "0xffff), k1 = ((s ^ 0x43210000) >> 16) | ((~s ^ 0xabcdffff) & 0xffff0000).");
        net("addSendTransform", "UONet::Connection_AddSendTransform", "call:sendSeed:9", 0x005feeb0L, true,
            "Appends an INetworkTransform to the connection's send chain (cipher).");
        login("selectShard", "LoginManager::SendSelectedShard", "str:LoginManager::SendSelectedShard:",
            0x00613790L, false, "Log \"LoginManager::SendSelectedShard\"; called by UOLua::SelectShard and the "
            + "0xA8 handler. Sends 0xA0 (shard index, 2 bytes) on the login connection and sets the title "
            + "\"UOSA - %s (%s) \".");
        login("buildSelectServer", "UOPacket::Build_0xA0_SelectServer", "call:selectShard:1", 0x0060e3e0L, false,
            "Creates the 0xA0 UONetMessageSend.");
        login("selectCharacter", "LoginManager::SelectCharacter", "call:luaSelectCharacter:9", 0x00611050L, false,
            "Called by UOLua::SelectCharacter: character list at LoginManager +0x44 (0x3c-byte entries); sends "
            + "the play-character packet (with the local IPv4) on the game connection.");
        login("deleteCharacter", "LoginManager::DeleteCharacter", "call:luaDeleteCharacter:10", 0x00610f70L,
            false, "Called by UOLua::DeleteCharacter: sends the delete-character packet on the game connection.");
        login("createCharacter", "LoginManager::CreateCharacter", "str:LoginManager::CreateCharacter",
            0x006111d0L, false, "Log \"LoginManager::CreateCharacter\".");

        // Packet handlers (LoginManager members registered on the login/game connection handler maps).
        handler("h_a8", 0xA8, "BritanniaList", 0x00611f10L, false,
            "Log \"LoginManager::Receive_BRITANNIA_LIST\": fills SystemData.Login.shardList; auto-selects a "
            + "shard (\"Sending request to connect to shard %s\").");
        handler("h_82", 0x82, "LoginFailed", 0x00612590L, false,
            "Log \"LoginManager: account login failed, reason %d\": sets SystemData.Login.statusCode. "
            + "Registered on both connections.");
        handler("h_8c", 0x8C, "UserServer", 0x00612670L, false,
            "Log \"LoginManager::Receive_USER_SERVER\" (server relay): disconnects the login connection, "
            + "connects the game connection to ip:port and calls ConnectToGameServer with the auth key.");
        handler("h_b9", 0xB9, "Features", 0x00612990L, true,
            "Reads a dword into a global flag word (supported features).");
        handler("h_a9", 0xA9, "CitiesAndChars", 0x00612a80L, false,
            "Log \"LoginManager::Receive_CITIES_AND_CHARS\".");
        handler("h_86", 0x86, "AllCharacters", 0x006130b0L, false,
            "Log \"LoginManager::Receive_ALL_CHARACTERS\" (character list update).");
        handler("h_1b", 0x1B, "LoginConfirm", 0x00611950L, false,
            "Log \"LoginManager::Receive_LOGIN_CONFIRM\" and \"Login succeeded to shard %s with character "
            + "%s\": enters the world.");
        handler("h_c8", 0xC8, "ViewRange", 0x00612a10L, true, "Reads one byte (client view range).");
        handler("h_53", 0x53, "ErrorCode", 0x00613430L, true,
            "Reads one byte into SystemData.Login.statusCode; disconnects unless it is 7.");
        handler("h_85", 0x85, "ChangeCharResult", 0x00615650L, false,
            "Log \"LoginManager: change char result, reason %d\" (character delete result).");
        handler("h_ee", 0xEE, "Unknown", 0x006133b0L, true,
            "Fixed 10-byte packet; stores its first word in LoginManager +0xa8 (initialised to 0xffff).");
        handler("h_e3", 0xE3, "EncryptionRequest", 0x006145c0L, true,
            "KR/EC key exchange: Crypto++ DH_Domain (DefaultSafePrime) and SHA256, replies with 0xE4 and "
            + "installs UOGameCipher2_C2S_Encrypt, UOGameCipher2_S2C_Decrypt and Huffman, then sends the "
            + "game login.");
        login("registerHandlers", "LoginManager::RegisterPacketHandlers", "regsite:0xA8", 0x0060ee10L, false,
            "Registers the LoginManager packet handlers (0xA8, 0x82, 0x8C on the login connection; 0x82, "
            + "0xB9, 0xA9, 0x86, 0x1B, 0xC8, 0x53, 0x85, 0xEE on the game connection).");
        net("handlerRegister", "UONet::PacketHandlerMap_Register", "register:0xA8", 0x00644290L, false,
            "this = connection +0x24 (handler map), args (packet id, callback shared_ptr); called by every "
            + "packet handler registration.");

        // Packet readers (first call in each handler).
        login("read_a8", "UOPacket::Read_0xA8_BritanniaList", "call:h_a8:0", 0x0060c7a0L, false, "Parses 0xA8.");
        login("read_82", "UOPacket::Read_0x82_LoginFailed", "call:h_82:0", 0x0060c3b0L, false, "Parses 0x82.");
        login("read_8c", "UOPacket::Read_0x8C_UserServer", "call:h_8c:0", 0x0060c6d0L, false,
            "Parses 0x8C (ip, port, auth key).");
        login("read_b9", "UOPacket::Read_0xB9_Features", "call:h_b9:0", 0x0060ca50L, true, "Parses 0xB9.");
        login("read_a9", "UOPacket::Read_0xA9_CitiesAndChars", "call:h_a9:0", 0x0060c890L, false, "Parses 0xA9.");
        login("read_86", "UOPacket::Read_0x86_AllCharacters", "call:h_86:0", 0x0060c5b0L, false, "Parses 0x86.");
        login("read_1b", "UOPacket::Read_0x1B_LoginConfirm", "call:h_1b:1", 0x0060bee0L, false, "Parses 0x1B.");
        login("read_c8", "UOPacket::Read_0xC8_ViewRange", "call:h_c8:0", 0x0060cac0L, true, "Reads 0xC8's byte.");
        login("read_53", "UOPacket::Read_0x53_ErrorCode", "call:h_53:0", 0x0060c050L, true, "Reads 0x53's byte.");
        login("read_85", "UOPacket::Read_0x85_ChangeCharResult", "call:h_85:0", 0x0060c4b0L, false, "Parses 0x85.");
        login("read_ee", "UOPacket::Read_0xEE_Unknown", "call:h_ee:0", 0x0060ccc0L, true,
            "Parses 0xEE (dword, byte, word, word).");
        login("read_e3", "UOPacket::Read_0xE3_EncryptionRequest", "call:h_e3:0", 0x0060cbc0L, true, "Parses 0xE3.");
        login("buildE4", "UOPacket::Build_0xE4_EncryptionResponse", "call:h_e3:42", 0x0060eb40L, false,
            "Creates the 0xE4 UONetMessageSend (variable length).");

        // Game connection setup and ciphers.
        login("disconnect", "LoginManager::Disconnect", "call:h_8c:2", 0x006105d0L, false,
            "Closes the login and game connections (vtable +0x50) and resets the login state; called by "
            + "UOLua::UndoLogin, the 0x8C/0x53/0xE3 handlers and GameServerConnection.");
        login("connectGame", "LoginManager::ConnectToGameServer", "call:h_8c:5", 0x00613e20L, false,
            "Sends the 4-byte auth key from 0x8C as the game seed, then installs UOGameCipher_C2S_Encrypt, "
            + "UOGameCipher_S2C_Decrypt and UOHuffmanCoding_S2C_Decompress and sends the game login.");
        login("sendGameLogin", "LoginManager::SendGameLogin", "call:connectGame:22", 0x00612790L, true,
            "Builds the 0x91 game server login (auth key LoginManager +0x58, account, password) and sends it "
            + "on the game connection.");
        net("addReceiveTransform", "UONet::Connection_AddReceiveTransform", "call:connectGame:16", 0x005fef40L,
            true, "Appends an INetworkTransform to the connection's receive chain (decrypt, decompress).");
        net("gameC2SInit", "UOGameCipher_C2S_Encrypt::Init", "call:connectGame:10", 0x00639ea0L, false,
            "Twofish key from the seed (4 copies), 256-byte pad 0..255 encrypted in place, counter +0x124c.");
        net("gameS2CInit", "UOGameCipher_S2C_Decrypt::Init", "call:connectGame:15", 0x0063be10L, false,
            "Builds the same Twofish pad and keeps a 16-byte digest of it (MD5) as the XOR table.");
        net("huffmanCtor", "UOHuffmanCoding_S2C_Decompress::UOHuffmanCoding_S2C_Decompress",
            "call:connectGame:18", 0x0063bf30L, false, "Stores the Huffman decompressor vftable.");
        net("gameC2STransform", "UOGameCipher_C2S_Encrypt::Transform", "vtable:UOGameCipher_C2S_Encrypt:0",
            0x0063bd10L, false, "Encrypts each outgoing byte with EncryptByte.");
        net("gameC2SByte", "UOGameCipher_C2S_Encrypt::EncryptByte", "call:gameC2STransform:1", 0x00639f90L,
            false, "XORs a byte with the Twofish pad; re-encrypts the pad every 256 bytes. ECLO ENC1 site.");
        net("gameS2CTransform", "UOGameCipher_S2C_Decrypt::Transform", "vtable:UOGameCipher_S2C_Decrypt:0",
            0x0063be90L, false, "XORs each incoming byte with the 16-byte digest, index counter % 16. ECLO "
            + "ENC2 site.");
        net("huffmanTransform", "UOHuffmanCoding_S2C_Decompress::Transform",
            "vtable:UOHuffmanCoding_S2C_Decompress:0", 0x0063c410L, false, "Decompresses incoming data.");
        net("loginTransform", "UOLoginCipher_C2S_Encrypt::Transform", "vtable:UOLoginCipher_C2S_Encrypt:0",
            0x0063c7b0L, false, "Classic login LFSR: byte ^= k0; k1 = (((k1 >> 1 | k0 << 31) ^ 0x1b92eb9c) "
            + ">> 1 | k0 << 31) ^ 0x1b92eb9d; k0 = (k0 >> 1 | old k1 << 31) ^ 0xb91e227f. ECLO ENC3 site.");
        net("cipher2C2SInit", "UOGameCipher2_C2S_Encrypt::Init", "call:h_e3:55", 0x0063a810L, true,
            "Creates the Crypto++ cipher from the negotiated key.");
        net("cipher2S2CInit", "UOGameCipher2_S2C_Decrypt::Init", "call:h_e3:60", 0x0063b900L, true,
            "Creates the Crypto++ cipher from the negotiated key.");
        net("cryptoAdapterTransform", "CryptoPPAdapter::Transform", "vtable:UOGameCipher2_C2S_Encrypt:0",
            0x006390b0L, false, "Shared slot 0 of UOGameCipher2_C2S_Encrypt and UOGameCipher2_S2C_Decrypt "
            + "(dtors store CryptoPPAdapter::vftable): runs the Crypto++ filter over the buffer.");

        // Packet framing and registry.
        net("registryFind", "UONet::PacketRegistry_Find", "call:netMessageSend:1", 0x005fd3c0L, false,
            "Hash lookup of a packet id in the packet registry (entry: dynamic flag, length, name).");
        net("registryInsert", "UONet::PacketRegistry_FindOrInsert", "registry:registryFind", 0x005fd050L, false,
            "Find, inserting a default entry when the id is missing.");
        net("registryBuild", "UONet::PacketRegistry_RegisterAll", "busiest:registryInsert", 0x005f8630L, false,
            "Registers the length of every packet id (dynamic flag, fixed length); the script output lists "
            + "the table.");
        net("extractPacket", "UONet::ExtractPacket", "import:registryFind:ntohs", 0x005ff6d0L, false,
            "Framing: reads the id byte, looks up its length (fixed, or the big-endian word at +1 for "
            + "variable packets) and cuts one UONetMessageReceive from the receive buffer.");
        net("processPackets", "BaseConnection::ProcessReceivedPackets", "caller:extractPacket", 0x005ff5b0L,
            true, "Loops ExtractPacket and DispatchPacket.");
        net("dispatchPacket", "BaseConnection::DispatchPacket", "call:processPackets:1", 0x005ff1b0L, true,
            "Asks the connection's vtable +0x2c filter, then dispatches the packet id to the handler map.");
        net("handlerDispatch", "UONet::PacketHandlerMap_Dispatch", "call:dispatchPacket:0", 0x006442f0L, true,
            "Calls the callbacks registered for the packet id.");
        net("onReceive", "BaseConnection::OnReceive", "vtable:BaseConnection:5", 0x005ff2d0L, true,
            "Reads socket data, runs the receive transform chain, then ProcessReceivedPackets.");
        net("gameFilter", "GameServerConnection::FilterPacket", "vtable:GameServerConnection:11", 0x00600720L,
            true, "Before encryption (state 1) only 0xE3 passes; other packets disconnect (LoginManager::"
            + "Disconnect). BaseConnection's slot returns true.");
        net("gameDisconnected", "GameServerConnection::OnDisconnected", "vtable:GameServerConnection:7",
            0x00600800L, true, "Sets SystemData.PlayState.isConnected to false and fires an event.");
        net("loginDisconnected", "LoginServerConnection::OnDisconnected", "vtable:LoginServerConnection:7",
            0x00600ca0L, true, "Same shape as GameServerConnection::OnDisconnected (event only).");

        // LoginConfig (LoginServersDefault.xml / LoginServersUser.xml).
        login("configLoad", "LoginConfig::LoadLoginServers", "str:LoginServersDefault.xml", 0x006052a0L, true,
            "Loads LoginServersDefault.xml then LoginServersUser.xml.");
        login("configSetInfo", "LoginConfig::SetInfoForLogin",
            "str:LoginConfig::SetInfoForLogin: Setting Login info for normal login", 0x006054b0L, false,
            "Log \"LoginConfig::SetInfoForLogin\": DNS lookup of the selected LoginServer, stores ip +0x9c, "
            + "port +0xa0.");

        // Globals.
        label("g_LoginManager", "read:luaSelectShard:1", 0x00e37304L, false,
            "LoginManager singleton (read by the UOLua login functions).");
        label("g_LoginConfig", "read:loginAccount:1", 0x00e37338L, false,
            "LoginConfig singleton (null without LoginServers*.xml); when set, LoginAccount uses its server.");
        label("g_LoginServerConnection", "conn:0xA8", 0x00e39954L, false,
            "shared_ptr<LoginServerConnection> object pointer.");
        label("g_LoginServerConnection_ctrl", "offset:g_LoginServerConnection:" + SHARED_PTR_CONTROL_OFFSET,
            0x00e39958L, false, "shared_ptr<LoginServerConnection> control block.");
        label("g_GameServerConnection", "conn:0xA9", 0x00e3994cL, false,
            "shared_ptr<GameServerConnection> object pointer.");
        label("g_GameServerConnection_ctrl", "offset:g_GameServerConnection:" + SHARED_PTR_CONTROL_OFFSET,
            0x00e39950L, false, "shared_ptr<GameServerConnection> control block.");
        label("g_ServerFeatureFlags", "write:h_b9:0", 0x00da987cL, true, "Dword read from packet 0xB9.");
        label("g_SelectedCharacterSlot", "write:luaSelectCharacter:0", 0x00e33554L, true,
            "Character slot stored by UOLua::SelectCharacter.");
        label("g_DefaultLoginHost1", "eclo:ip1", 0x00cb78ccL, false,
            "Hardcoded login host 1 (16-byte slot), ECLO IP patch target.");
        label("g_DefaultLoginHost2", "eclo:ip2", 0x00cb78dcL, false,
            "Hardcoded login host 2 (16-byte slot), ECLO IP patch target (TOL clients).");
    }

    private Spec spec(String key, Kind kind, String path, String tag, String locator, long expected,
            boolean probable, String evidence) {
        Spec s = new Spec();
        s.key = key;
        s.kind = kind;
        s.path = path;
        s.tag = tag;
        s.locator = locator;
        s.expected = expected;
        s.probable = probable;
        s.evidence = evidence;
        specs.add(s);
        return s;
    }

    private void anchor(String key, String locator, long expected) {
        spec(key, Kind.ANCHOR, null, null, locator, expected, false, null);
    }

    private void login(String key, String path, String locator, long expected, boolean probable, String evidence) {
        spec(key, Kind.FUNCTION, path, TAG_LOGIN, locator, expected, probable, evidence);
    }

    private void net(String key, String path, String locator, long expected, boolean probable, String evidence) {
        spec(key, Kind.FUNCTION, path, TAG_NET, locator, expected, probable, evidence);
    }

    private void handler(String key, int id, String name, long expected, boolean probable, String evidence) {
        spec(key, Kind.FUNCTION, String.format("UOPacket::Handle_0x%02X_%s", id, name), TAG_LOGIN,
            String.format("handler:0x%02X", id), expected, probable, evidence);
    }

    private void label(String name, String locator, long expected, boolean probable, String evidence) {
        spec(name, Kind.LABEL, name, TAG_LOGIN, locator, expected, probable, evidence);
    }

    // ---- Analysis ----

    private void analyze(PrintWriter out, boolean apply) throws Exception {
        testedBuild = TESTED_SHA256.equalsIgnoreCase(currentProgram.getExecutableSHA256());
        defineSpecs();
        scanRegistrations();
        for (Spec s : specs) {
            resolve(s);
        }
        if (located.containsKey("registryBuild") && located.containsKey("registryInsert")) {
            readPacketTable(located.get("registryBuild"), located.get("registryInsert"));
        }
        List<Object[]> ecloComments = ecloSites();
        if (apply) {
            for (Spec s : specs) {
                Address a = located.get(s.key);
                if (a == null || s.kind == Kind.ANCHOR) {
                    continue;
                }
                if (s.kind == Kind.FUNCTION) {
                    applyFunction(s, a);
                } else {
                    applyLabel(s, a);
                }
            }
            for (Object[] c : ecloComments) {
                applyComment((Address) c[0], (String) c[1]);
            }
        }
        report(out, apply);
    }

    private void resolve(Spec s) {
        String result = null;
        Address a = null;
        try {
            a = locate(s.locator);
        } catch (RuntimeException failure) {
            result = failure.getMessage();
        }
        Address expected = toAddr(s.expected);
        if (a == null) {
            if (testedBuild) {
                a = expected;
                how.put(s.key, "fallback to tested address (" + (result == null ? "not found" : result) + ")");
                anomalies.add("`" + s.key + "` locator `" + s.locator + "` failed: " + result
                    + "; used the tested address " + expected);
            } else {
                anomalies.add("`" + s.key + "` locator `" + s.locator + "` failed: " + result + "; skipped");
                return;
            }
        } else {
            how.put(s.key, s.locator);
            if (testedBuild && !a.equals(expected)) {
                anomalies.add("`" + s.key + "` located at " + a + ", tested address " + expected + "; skipped");
                return;
            }
        }
        if (s.kind == Kind.FUNCTION && getFunctionAt(a) == null) {
            anomalies.add("`" + s.key + "` at " + a + " is not a function entry; skipped");
            return;
        }
        located.put(s.key, a);
    }

    private Address locate(String locator) {
        String[] p = locator.split(":", 2);
        String kind = p[0];
        String rest = p.length > 1 ? p[1] : "";
        String[] r = rest.split(":");
        switch (kind) {
            case "str":
                return uniqueStringUser(rest);
            case "call":
                return nthCall(need(r[0]), Integer.parseInt(r[1]));
            case "handler":
                return registration(parseId(rest)).handler;
            case "regsite":
                return registration(parseId(rest)).owner.getEntryPoint();
            case "register":
                return registration(parseId(rest)).register;
            case "conn":
                return registration(parseId(rest)).connection;
            case "vtable":
                return vtableSlot(r[0], Integer.parseInt(r[1]));
            case "read":
                return nthData(need(r[0]), Integer.parseInt(r[1]), false);
            case "write":
                return nthData(need(r[0]), Integer.parseInt(r[1]), true);
            case "offset":
                return need(r[0]).add(Long.parseLong(r[1]));
            case "caller":
                return uniqueCaller(need(r[0]), null, null);
            case "othercaller":
                return uniqueCaller(need(r[0]), need(r[1]), null);
            case "import":
                return uniqueCaller(need(r[0]), null, r[1]);
            case "registry":
                return registryInsert(need(r[0]));
            case "busiest":
                return busiestCaller(need(r[0]));
            case "eclo":
                return loginHostSlot(rest);
            default:
                throw new IllegalArgumentException("unknown locator " + locator);
        }
    }

    private Address need(String key) {
        Address a = located.get(key);
        if (a == null) {
            throw new IllegalStateException("depends on unresolved `" + key + "`");
        }
        return a;
    }

    private int parseId(String text) {
        return Integer.parseInt(text.replace("0x", ""), HEX_RADIX);
    }

    // ---- Locators ----

    private Address uniqueStringUser(String prefix) {
        if (strings == null) {
            strings = new ArrayList<>();
            DataIterator it = currentProgram.getListing().getDefinedData(true);
            while (it.hasNext() && !monitor.isCancelled()) {
                Data d = it.next();
                if (d.hasStringValue()) {
                    strings.add(d);
                }
            }
        }
        Data match = null;
        for (Data d : strings) {
            String value = StringDataInstance.getStringDataInstance(d).getStringValue();
            if (value != null && value.startsWith(prefix)) {
                if (match != null) {
                    throw new IllegalStateException("several strings start with \"" + prefix + "\"");
                }
                match = d;
            }
        }
        if (match == null) {
            throw new IllegalStateException("no string starts with \"" + prefix + "\"");
        }
        Set<Function> users = new LinkedHashSet<>();
        for (Reference ref : getReferencesTo(match.getAddress())) {
            Function f = getFunctionContaining(ref.getFromAddress());
            if (f != null) {
                users.add(f);
            }
        }
        if (users.size() != 1) {
            throw new IllegalStateException(users.size() + " functions use \"" + prefix + "\"");
        }
        return users.iterator().next().getEntryPoint();
    }

    private List<Instruction> body(Address entry) {
        Function f = getFunctionAt(entry);
        if (f == null) {
            throw new IllegalStateException("no function at " + entry);
        }
        List<Instruction> list = new ArrayList<>();
        for (Instruction ins : currentProgram.getListing().getInstructions(f.getBody(), true)) {
            list.add(ins);
        }
        return list;
    }

    private Address nthCall(Address parent, int ordinal) {
        int n = 0;
        for (Instruction ins : body(parent)) {
            if (ins.getFlowType().isCall() && ins.getFlows().length > 0) {
                if (n++ == ordinal) {
                    Address target = ins.getFlows()[0];
                    if (!target.isMemoryAddress()) {
                        throw new IllegalStateException("call " + ordinal + " in " + parent + " is external");
                    }
                    return target;
                }
            }
        }
        throw new IllegalStateException(parent + " has fewer than " + (ordinal + 1) + " calls");
    }

    private boolean isCodeAddress(Address a) {
        MemoryBlock block = currentProgram.getMemory().getBlock(a);
        return block == null || block.isExecute();
    }

    private Address nthData(Address parent, int ordinal, boolean write) {
        int n = 0;
        for (Instruction ins : body(parent)) {
            for (Reference ref : ins.getReferencesFrom()) {
                Address to = ref.getToAddress();
                boolean kind = write ? ref.getReferenceType().isWrite() : ref.getReferenceType().isRead();
                if (!kind || !to.isMemoryAddress() || isCodeAddress(to)) {
                    continue;
                }
                Data data = getDataContaining(to);
                if (data == null || !data.hasStringValue()) {
                    if (n++ == ordinal) {
                        return to;
                    }
                }
            }
        }
        throw new IllegalStateException(parent + " has fewer than " + (ordinal + 1) + " data accesses");
    }

    private Set<Function> callers(Address callee) {
        Set<Function> set = new LinkedHashSet<>();
        for (Reference ref : getReferencesTo(callee)) {
            if (ref.getReferenceType().isCall()) {
                Function f = getFunctionContaining(ref.getFromAddress());
                if (f != null) {
                    set.add(f);
                }
            }
        }
        return set;
    }

    private boolean callsImport(Function f, String name) {
        for (Instruction ins : body(f.getEntryPoint())) {
            if (ins.getFlowType().isCall()) {
                for (Address t : ins.getFlows()) {
                    Symbol s = getSymbolAt(t);
                    Function tf = getFunctionAt(t);
                    if (tf != null && tf.isExternal() && tf.getName().equals(name)
                            || s != null && s.isExternal() && s.getName().equals(name)) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    private Address uniqueCaller(Address callee, Address excluded, String importName) {
        List<Function> found = new ArrayList<>();
        for (Function f : callers(callee)) {
            if (excluded != null && f.getEntryPoint().equals(excluded)) {
                continue;
            }
            if (importName != null && !callsImport(f, importName)) {
                continue;
            }
            found.add(f);
        }
        if (found.size() != 1) {
            throw new IllegalStateException(found.size() + " matching callers of " + callee);
        }
        return found.get(0).getEntryPoint();
    }

    private int callCount(Function from, Address callee) {
        int n = 0;
        for (Reference ref : getReferencesTo(callee)) {
            if (ref.getReferenceType().isCall() && from.getBody().contains(ref.getFromAddress())) {
                n++;
            }
        }
        return n;
    }

    private Address busiestCaller(Address callee) {
        for (Function f : callers(callee)) {
            if (callCount(f, callee) >= MIN_REGISTRY_ENTRIES) {
                return f.getEntryPoint();
            }
        }
        throw new IllegalStateException("no function calls " + callee + " " + MIN_REGISTRY_ENTRIES + " times");
    }

    private Address registryInsert(Address find) {
        List<Address> found = new ArrayList<>();
        for (Function f : callers(find)) {
            try {
                busiestCaller(f.getEntryPoint());
                found.add(f.getEntryPoint());
            } catch (IllegalStateException none) {
                // Not called by a table builder.
            }
        }
        if (found.size() != 1) {
            throw new IllegalStateException(found.size() + " callers of " + find + " are table inserts");
        }
        return found.get(0);
    }

    private Address vtableSlot(String className, int slot) {
        SymbolTable table = currentProgram.getSymbolTable();
        Namespace ns = table.getNamespace(className, currentProgram.getGlobalNamespace());
        if (ns == null) {
            throw new IllegalStateException("no class " + className);
        }
        List<Symbol> vt = table.getSymbols(VFTABLE, ns);
        if (vt.size() != 1) {
            throw new IllegalStateException(vt.size() + " vftables in " + className);
        }
        try {
            long pointer = currentProgram.getMemory().getInt(vt.get(0).getAddress().add((long) slot * POINTER_SIZE))
                & DWORD_MASK;
            return toAddr(pointer);
        } catch (Exception failure) {
            throw new IllegalStateException("unreadable vftable slot " + slot + " of " + className);
        }
    }

    // ---- Handler registrations ----

    private void scanRegistrations() {
        SymbolTable table = currentProgram.getSymbolTable();
        for (Symbol s : table.getSymbols(VFTABLE)) {
            String owner = s.getParentNamespace().getName(true);
            if (!owner.startsWith(CALLBACK_PREFIX) || !owner.endsWith(CALLBACK_OWNER) || owner.contains(COUNTED_PREFIX)) {
                continue;
            }
            for (Reference ref : getReferencesTo(s.getAddress())) {
                Instruction ins = getInstructionAt(ref.getFromAddress());
                Function f = getFunctionContaining(ref.getFromAddress());
                if (ins == null || f == null) {
                    continue;
                }
                Registration reg = scanSite(ins, f);
                if (reg != null) {
                    registrations.add(reg);
                } else {
                    anomalies.add("Callback vftable store at " + ins.getAddress() + " not decoded");
                }
            }
        }
    }

    private Registration scanSite(Instruction store, Function owner) {
        Registration reg = new Registration();
        reg.site = store.getAddress();
        reg.owner = owner;
        List<Instruction> seen = new ArrayList<>();
        Instruction ins = store;
        for (int i = 0; i < REGISTRATION_WINDOW && ins != null; i++, ins = ins.getNext()) {
            seen.add(ins);
            String m = ins.getMnemonicString();
            if (reg.handler == null && m.equals("MOV") && displacement(ins, 0) == CALLBACK_HANDLER_OFFSET
                    && ins.getScalar(1) != null) {
                Address h = toAddr(ins.getScalar(1).getUnsignedValue());
                if (getFunctionAt(h) != null) {
                    reg.handler = h;
                }
            }
            if (reg.handler != null && ins.getFlowType().isCall() && ins.getFlows().length > 0 && seen.size() >= 3) {
                Instruction lea = seen.get(seen.size() - 2);
                Instruction push = seen.get(seen.size() - 3);
                if (lea.getMnemonicString().equals("LEA") && displacement(lea, 1) == CONNECTION_HANDLER_MAP_OFFSET
                        && push.getMnemonicString().equals("PUSH") && push.getScalar(0) != null) {
                    reg.register = ins.getFlows()[0];
                    reg.id = (int) push.getScalar(0).getUnsignedValue() & BYTE_MASK;
                    reg.connection = connectionGlobal(seen, baseRegister(lea));
                    return reg.connection == null ? null : reg;
                }
            }
        }
        return null;
    }

    private Register baseRegister(Instruction ins) {
        for (Object o : ins.getOpObjects(1)) {
            if (o instanceof Register) {
                return (Register) o;
            }
        }
        return null;
    }

    private long displacement(Instruction ins, int operand) {
        long value = -1;
        boolean register = false;
        for (Object o : ins.getOpObjects(operand)) {
            if (o instanceof Scalar) {
                value = ((Scalar) o).getSignedValue();
            } else if (o instanceof Register) {
                register = true;
            }
        }
        return register ? value : -1;
    }

    // The last MOV of the LEA base register from an absolute global before the call.
    private Address connectionGlobal(List<Instruction> seen, Register base) {
        int stop = Math.max(0, seen.size() - CONNECTION_LOOKBACK);
        for (int i = seen.size() - 1; i >= stop; i--) {
            Instruction ins = seen.get(i);
            if (ins.getMnemonicString().equals("MOV") && base != null && base.equals(ins.getRegister(0))) {
                for (Reference ref : ins.getReferencesFrom()) {
                    if (ref.getReferenceType().isRead() && ref.getToAddress().isMemoryAddress()) {
                        return ref.getToAddress();
                    }
                }
                return null;
            }
        }
        return null;
    }

    private Registration registration(int id) {
        Registration found = null;
        for (Registration r : registrations) {
            if (r.id != id) {
                continue;
            }
            if (found != null && !found.handler.equals(r.handler)) {
                throw new IllegalStateException(String.format("packet 0x%02X has several handlers", id));
            }
            if (found == null) {
                found = r;
            }
        }
        if (found == null) {
            throw new IllegalStateException(String.format("no LoginManager handler for packet 0x%02X", id));
        }
        return found;
    }

    // ---- Packet length table ----

    // Emulates the straight-line table builder: each entry is either stored on the stack
    // (id byte, dynamic flag, length) or produced by a helper called with EAX = flag,
    // ECX = length, then passed to FindOrInsert with EAX pointing at the id byte.
    private void readPacketTable(Address builder, Address insert) {
        Map<String, Long> regs = new HashMap<>();
        Map<Long, Long> stack = new HashMap<>();
        long depth = 0;
        long idSlot = Long.MIN_VALUE;
        Long helperFlag = null;
        Long helperLength = null;
        int unresolved = 0;
        for (Instruction ins : body(builder)) {
            String m = ins.getMnemonicString();
            Register dst = ins.getNumOperands() > 0 ? ins.getRegister(0) : null;
            if (m.equals("PUSH")) {
                depth += POINTER_SIZE;
            } else if (m.equals("XOR") && dst != null && dst.equals(ins.getRegister(1))) {
                regs.put(root(dst), 0L);
            } else if (m.equals("LEA") && dst != null) {
                Register src = baseRegister(ins);
                long disp = displacement(ins, 1);
                if (src != null && src.getName().equals("ESP")) {
                    if (root(dst).equals("EAX")) {
                        idSlot = disp - depth;
                    }
                    regs.remove(root(dst));
                } else if (src != null && regs.containsKey(root(src)) && disp >= 0) {
                    regs.put(root(dst), regs.get(root(src)) + disp);
                } else {
                    regs.remove(root(dst));
                }
            } else if (m.equals("MOV") && dst != null) {
                Long v = immediate(ins, regs);
                if (v != null) {
                    regs.put(root(dst), v);
                } else {
                    regs.remove(root(dst));
                }
            } else if (m.equals("MOV") && ins.getDefaultOperandRepresentation(0).contains("[ESP")) {
                Long v = immediate(ins, regs);
                stack.put(displacement(ins, 0) - depth, v == null ? Long.MIN_VALUE : v);
            } else if (ins.getFlowType().isCall()) {
                Address t = ins.getFlows().length > 0 ? ins.getFlows()[0] : null;
                if (insert.equals(t)) {
                    Long id = stack.get(idSlot);
                    long slot = idSlot;
                    if (!known(id)) {
                        slot = tripleSlot(stack);
                        id = stack.get(slot);
                    }
                    Long flag = helperFlag != null ? helperFlag : stack.get(slot + ENTRY_FLAG_OFFSET);
                    Long length = helperLength != null ? helperLength : stack.get(slot + ENTRY_LENGTH_OFFSET);
                    if (known(id) && known(flag) && known(length)) {
                        int key = (int) (id & BYTE_MASK);
                        String entry = flag != 0 ? "variable" : Long.toString(length);
                        if (packetTable.containsKey(key)) {
                            reregistered.add(key);
                        }
                        packetTable.put(key, entry);
                    } else {
                        unresolved++;
                    }
                    helperFlag = null;
                    helperLength = null;
                    stack.clear();
                } else if (regs.containsKey("EAX") && regs.containsKey("ECX")) {
                    helperFlag = regs.get("EAX");
                    helperLength = regs.get("ECX");
                }
                depth = 0;
                regs.remove("EAX");
                regs.remove("ECX");
                regs.remove("EDX");
            } else if (m.equals("ADD") && dst != null && dst.getName().equals("ESP") && ins.getScalar(1) != null) {
                depth -= ins.getScalar(1).getUnsignedValue();
            } else if (dst != null) {
                regs.remove(root(dst));
            }
        }
        if (unresolved > 0) {
            anomalies.add(unresolved + " packet registry entries not decoded");
        }
    }

    private boolean known(Long v) {
        return v != null && v != Long.MIN_VALUE;
    }

    private long tripleSlot(Map<Long, Long> stack) {
        for (Long k : stack.keySet()) {
            if (stack.containsKey(k + ENTRY_FLAG_OFFSET) && stack.containsKey(k + ENTRY_LENGTH_OFFSET)) {
                return k;
            }
        }
        return Long.MIN_VALUE;
    }

    private String root(Register r) {
        Register b = r.getBaseRegister();
        return (b != null ? b : r).getName();
    }

    private Long immediate(Instruction ins, Map<String, Long> regs) {
        Object[] src = ins.getOpObjects(1);
        if (src.length == 1 && src[0] instanceof Scalar) {
            return ((Scalar) src[0]).getUnsignedValue();
        }
        Register r = ins.getRegister(1);
        if (r != null && regs.containsKey(root(r))) {
            long v = regs.get(root(r));
            return r.getBitLength() == Byte.SIZE ? v & BYTE_MASK : v;
        }
        return null;
    }

    // ---- ECLO patch sites ----

    private List<Long> find(int[] pattern, Address from) {
        byte[] bytes = new byte[pattern.length];
        byte[] mask = new byte[pattern.length];
        for (int i = 0; i < pattern.length; i++) {
            bytes[i] = (byte) (pattern[i] == ANY ? 0 : pattern[i]);
            mask[i] = (byte) (pattern[i] == ANY ? 0 : BYTE_MASK);
        }
        List<Long> hits = new ArrayList<>();
        Memory memory = currentProgram.getMemory();
        Address start = from != null ? from : memory.getMinAddress();
        while (start != null) {
            Address hit = memory.findBytes(start, bytes, mask, true, monitor);
            if (hit == null) {
                break;
            }
            hits.add(hit.getOffset());
            start = hit.next();
        }
        return hits;
    }

    private boolean matches(Address at, int[] pattern) {
        try {
            for (int i = 0; i < pattern.length; i++) {
                if (pattern[i] != ANY && (getByte(at.add(i)) & BYTE_MASK) != pattern[i]) {
                    return false;
                }
            }
            return true;
        } catch (Exception failure) {
            return false;
        }
    }

    private int word(Address at) throws Exception {
        return currentProgram.getMemory().getShort(at) & WORD_MASK;
    }

    private int[] titleSignature() {
        byte[] text = TITLE_FORMAT.getBytes(StandardCharsets.US_ASCII);
        int[] sig = new int[TITLE_GUARD.length + text.length + TITLE_TRAILER];
        System.arraycopy(TITLE_GUARD, 0, sig, 0, TITLE_GUARD.length);
        for (int i = 0; i < text.length; i++) {
            sig[TITLE_GUARD.length + i] = text[i];
        }
        return sig;
    }

    private Address loginHostSlot(String which) {
        int slot = Integer.parseInt(which.replace("ip", "")) - FIRST_IP_SLOT;
        if (slot < 0 || slot >= IP_SLOTS) {
            throw new IllegalArgumentException("unknown host slot " + which);
        }
        int[] sig = titleSignature();
        List<Long> hits = find(sig, null);
        if (hits.size() != 1) {
            throw new IllegalStateException(hits.size() + " ECLO title signatures");
        }
        return toAddr(hits.get(0) + sig.length + (long) slot * IP_SLOT_SIZE);
    }

    private String ascii(Address at) {
        StringBuilder text = new StringBuilder();
        try {
            for (int i = 0; i < IP_SLOT_SIZE; i++) {
                int c = getByte(at.add(i)) & BYTE_MASK;
                if (c == 0) {
                    break;
                }
                text.append((char) c);
            }
        } catch (Exception failure) {
            return "?";
        }
        return text.toString();
    }

    private List<Object[]> ecloSites() throws Exception {
        List<Object[]> comments = new ArrayList<>();
        List<Long> port1 = find(PORT1_PATTERN, null);
        if (port1.isEmpty()) {
            anomalies.add("ECLO port1 pattern not found");
        } else {
            Address p1 = toAddr(port1.get(0));
            long ip = currentProgram.getMemory().getInt(p1.add(PORT1_IP_IMM)) & DWORD_MASK;
            Address w1 = p1.add(PORT1_WRITE);
            Address imm1 = w1.add(PORT_IMM_OFFSET);
            site(comments, "PORT1", w1, String.format("ECLO port patch 1 rewrites the imm16 at %s (default %d): "
                + "the port of the internal test address %s (previous instruction, not patched by ECLO), used "
                + "only when app +0x1c9 is set.", imm1, word(imm1), dotted(ip)), port1.size());
            List<Long> port2 = find(PORT2_PATTERN, p1.add(1));
            if (port2.isEmpty()) {
                anomalies.add("ECLO port2 pattern not found after port1");
            } else {
                Address w2 = toAddr(port2.get(0)).add(PORT2_WRITE);
                Address imm2 = w2.add(PORT_IMM_OFFSET);
                site(comments, "PORT2", w2, String.format("ECLO port patch 2 (first port2 match after port1) "
                    + "rewrites the imm16 at %s (default %d): the hardcoded-host port when rand() is odd.", imm2, word(imm2)),
                    port2.size());
                Address jnz = w2.add(PORT_WRITE_LENGTH);
                if (matches(jnz, PORT3_PREFIX)) {
                    Address w3 = jnz.add(JNZ_SHORT_LENGTH);
                    Address imm3 = w3.add(PORT_IMM_OFFSET);
                    site(comments, "PORT3", w3, String.format("ECLO port patch 3 (TOL clients) rewrites the "
                        + "imm16 at %s (default %d): the hardcoded-host port when rand() is even.", imm3, word(imm3)),
                        1);
                } else {
                    anomalies.add("ECLO TOL port3 bytes not found after port2");
                }
            }
        }
        for (int slot = FIRST_IP_SLOT; slot <= IP_SLOTS; slot++) {
            try {
                Address ipSlot = loginHostSlot("ip" + slot);
                site(comments, "IP" + slot, ipSlot, String.format("ECLO IP patch %d rewrites this 16-byte host "
                    + "slot (default \"%s\"; host %d is used when rand() is %s). Bypassed when LoginServers*.xml provides "
                    + "a LoginConfig.", slot, ascii(ipSlot), slot, slot == FIRST_IP_SLOT ? "odd" : "even"), 1);
            } catch (IllegalStateException failure) {
                anomalies.add("ECLO IP slot " + slot + ": " + failure.getMessage());
            }
        }
        encryption(comments, "ENC1", ENC1_PATTERN, "game client-to-server Twofish pad XOR");
        encryption(comments, "ENC2", ENC2_PATTERN, "game server-to-client digest XOR");
        encryption(comments, "ENC3", ENC3_PATTERN, "login client-to-server key XOR");
        return comments;
    }

    private void encryption(List<Object[]> comments, String name, int[] pattern, String what) {
        List<Long> hits = find(pattern, null);
        if (hits.size() != 1) {
            anomalies.add("ECLO " + name + " pattern matches " + hits.size() + " times");
            return;
        }
        site(comments, name, toAddr(hits.get(0)), "ECLO disable-encryption " + name + " (commented out in ECLO): "
            + "NOP this XOR (" + what + ").", hits.size());
    }

    private void site(List<Object[]> comments, String name, Address at, String text, int matches) {
        Function f = getFunctionContaining(at);
        ecloRows.add("| " + name + " | " + at + " | " + (f == null ? "-" : f.getName(true)) + " | " + matches
            + " | " + text.replace("|", "\\|") + " |");
        comments.add(new Object[] {at, COMMENT_PREFIX + text});
    }

    private String dotted(long ip) {
        // The address struct holds the IPv4 in host order (the port beside it is not swapped either).
        return String.format("%d.%d.%d.%d", (ip >> (3 * Byte.SIZE)) & BYTE_MASK, (ip >> (2 * Byte.SIZE)) & BYTE_MASK,
            (ip >> Byte.SIZE) & BYTE_MASK, ip & BYTE_MASK);
    }

    // ---- Apply ----

    private String plate(Spec s) {
        return COMMENT_PREFIX + s.tag + " `" + s.path + "`: " + (s.probable ? PROBABLE : "") + s.evidence
            + " Located by `" + how.get(s.key) + "`.";
    }

    private void applyFunction(Spec s, Address a) throws Exception {
        Function function = getFunctionAt(a);
        int split = s.path.lastIndexOf(SCOPE);
        String name = split < 0 ? s.path : s.path.substring(split + SCOPE.length());
        Namespace namespace = split < 0 ? currentProgram.getGlobalNamespace() : namespace(s.path.substring(0, split));
        Symbol symbol = function.getSymbol();
        if (!(symbol.getName().equals(name) && symbol.getParentNamespace().equals(namespace))) {
            if (symbol.getSource() == SourceType.DEFAULT) {
                try {
                    symbol.setNameAndNamespace(name, namespace, SourceType.ANALYSIS);
                } catch (DuplicateNameException duplicate) {
                    String unique = name + "_" + a.toString();
                    symbol.setNameAndNamespace(unique, namespace, SourceType.ANALYSIS);
                    anomalies.add("`" + s.path + "` already exists; " + a + " named `" + unique + "`");
                }
                count("functions renamed");
            } else {
                kept.add(a + " `" + symbol.getName(true) + "` (" + symbol.getSource() + "), not renamed to `"
                    + s.path + "`");
            }
        }
        boolean tagged = false;
        for (FunctionTag tag : function.getTags()) {
            tagged |= tag.getName().equals(s.tag);
        }
        if (!tagged) {
            function.addTag(s.tag);
            count("tags added");
        }
        String comment = function.getComment();
        if (comment == null || comment.isBlank()) {
            function.setComment(plate(s));
            count("plate comments set");
        } else if (comment.startsWith(COMMENT_PREFIX) && !comment.equals(plate(s))) {
            function.setComment(plate(s));
            count("plate comments updated");
        }
    }

    private void applyLabel(Spec s, Address a) throws Exception {
        SymbolTable table = currentProgram.getSymbolTable();
        for (Symbol existing : table.getSymbols(a)) {
            if (existing.getName().equals(s.path)) {
                if (!existing.isPrimary()) {
                    existing.setPrimary();
                    count("labels made primary");
                }
                commentLabel(s, a);
                return;
            }
        }
        Symbol primary = table.getPrimarySymbol(a);
        if (primary != null && primary.getSource() != SourceType.DEFAULT) {
            kept.add(a + " label `" + primary.getName(true) + "` (" + primary.getSource() + "), not relabelled `"
                + s.path + "`");
            return;
        }
        Symbol created = createLabel(a, s.path, true, SourceType.ANALYSIS);
        if (!created.isPrimary()) {
            created.setPrimary();
        }
        count("labels created");
        commentLabel(s, a);
    }

    private void commentLabel(Spec s, Address a) {
        Listing listing = currentProgram.getListing();
        String text = COMMENT_PREFIX + (s.probable ? PROBABLE : "") + s.evidence + " Located by `" + how.get(s.key) + "`.";
        String existing = listing.getComment(CodeUnit.PLATE_COMMENT, a);
        if (existing == null || existing.isBlank()) {
            listing.setComment(a, CodeUnit.PLATE_COMMENT, text);
            count("label comments set");
        } else if (existing.startsWith(COMMENT_PREFIX) && !existing.equals(text)) {
            listing.setComment(a, CodeUnit.PLATE_COMMENT, text);
            count("label comments updated");
        }
    }

    private void applyComment(Address at, String text) {
        Listing listing = currentProgram.getListing();
        String existing = listing.getComment(CodeUnit.EOL_COMMENT, at);
        if (existing == null || existing.isBlank()) {
            listing.setComment(at, CodeUnit.EOL_COMMENT, text);
            count("ECLO comments set");
        } else if (existing.startsWith(COMMENT_PREFIX) && !existing.equals(text)) {
            listing.setComment(at, CodeUnit.EOL_COMMENT, text);
            count("ECLO comments updated");
        } else if (!existing.equals(text)) {
            kept.add(at + " EOL comment kept (differs from the ECLO note)");
        }
    }

    private Namespace namespace(String path) throws Exception {
        SymbolTable table = currentProgram.getSymbolTable();
        Namespace namespace = currentProgram.getGlobalNamespace();
        for (String part : path.split(SCOPE)) {
            Namespace child = table.getNamespace(part, namespace);
            if (child == null) {
                child = table.createNameSpace(namespace, part, SourceType.ANALYSIS);
                count("namespaces created");
            }
            namespace = child;
        }
        return namespace;
    }

    private void count(String change) {
        changes.merge(change, 1, Integer::sum);
    }

    // ---- Report ----

    private void report(PrintWriter out, boolean apply) {
        out.println("# NameLoginCode report");
        out.println();
        out.println("Program: " + currentProgram.getName());
        out.println("SHA-256: " + currentProgram.getExecutableSHA256() + (testedBuild ? " (tested build)" : " (untested build)"));
        out.println("Mode: " + (apply ? "apply" : "report only"));
        out.println("Changes: " + (apply ? changes.toString() : "none"));
        out.println();
        out.println("## Names");
        out.println();
        out.println("| Address | Name | Tag | Current | Locator |");
        out.println("|---|---|---|---|---|");
        for (Spec s : specs) {
            Address a = located.get(s.key);
            if (s.kind == Kind.ANCHOR) {
                Function f = a == null ? null : getFunctionAt(a);
                out.println("| " + (a == null ? "-" : a) + " | (anchor `" + s.key + "`) | - | "
                    + (f == null ? "-" : f.getName(true)) + " | `" + s.locator + "` |");
                continue;
            }
            String current = "-";
            if (a != null) {
                if (s.kind == Kind.FUNCTION) {
                    current = getFunctionAt(a).getName(true);
                } else {
                    Symbol p = currentProgram.getSymbolTable().getPrimarySymbol(a);
                    current = p == null ? "-" : p.getName(true);
                }
            }
            out.println("| " + (a == null ? "-" : a) + " | `" + s.path + "`" + (s.probable ? " (probable)" : "")
                + " | " + s.tag + " | " + current + " | `" + (a == null ? s.locator : how.get(s.key)) + "` |");
        }
        out.println();
        out.println("## LoginManager packet handler registrations");
        out.println();
        out.println("| Site | Registering function | Packet | Connection global | Handler |");
        out.println("|---|---|---|---|---|");
        for (Registration r : registrations) {
            Function h = getFunctionAt(r.handler);
            out.println(String.format("| %s | %s | 0x%02X | %s | %s %s |", r.site, r.owner.getName(true), r.id,
                r.connection, r.handler, h == null ? "" : h.getName(true)));
        }
        out.println();
        out.println("## Packet length table");
        out.println();
        out.println(packetTable.size() + " ids registered (\"variable\": 2-byte big-endian length after the id). "
            + "Registered twice (last wins): " + hexList(reregistered) + ".");
        out.println();
        StringBuilder line = new StringBuilder();
        for (int id = 0; id < PACKET_IDS; id++) {
            if (packetTable.containsKey(id)) {
                line.append(String.format("0x%02X=%s ", id, packetTable.get(id)));
            }
            if ((id + 1) % HEX_RADIX == 0 && line.length() > 0) {
                out.println("    " + line.toString().trim());
                line.setLength(0);
            }
        }
        out.println();
        out.println("## ECLO patch sites");
        out.println();
        out.println("| Patch | Address | Function | Matches | Note |");
        out.println("|---|---|---|---|---|");
        ecloRows.forEach(out::println);
        out.println();
        out.println("## Kept (not DEFAULT, not changed)");
        out.println();
        kept.forEach(k -> out.println("- " + k));
        out.println();
        out.println("## Anomalies");
        out.println();
        anomalies.forEach(a -> out.println("- " + a));
    }

    private String hexList(Set<Integer> ids) {
        List<String> parts = new ArrayList<>();
        for (int id : ids) {
            parts.add(String.format("0x%02X", id));
        }
        return parts.isEmpty() ? "none" : String.join(", ", parts);
    }
}
