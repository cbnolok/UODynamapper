/* Macros normally supplied by windows.h, objbase.h and _mingw.h, which are stubbed here.
 * Base types (UINT, HRESULT, GUID, RECT, ...) come from Ghidra's windows_vs12_32.gdt. */

/* WinAPI keywords and constants */
#define CONST const
#define VOID void
#define FAR
#define NEAR
#define WINAPI __stdcall
#define WINAPIV __cdecl
#define CALLBACK __stdcall
#define EXTERN_C extern
#define WINBOOL BOOL
#define TRUE 1
#define FALSE 0
#define MAX_PATH 260
#define LF_FACESIZE 32
#define UINT_MAX 0xffffffff
#define __MSABI_LONG(x) x##l
#define MAKE_HRESULT(sev,fac,code) ((HRESULT)(((unsigned long)(sev)<<31)|((unsigned long)(fac)<<16)|((unsigned long)(code))))
#define MAKEFOURCC(ch0,ch1,ch2,ch3) ((DWORD)(BYTE)(ch0)|((DWORD)(BYTE)(ch1)<<8)|((DWORD)(BYTE)(ch2)<<16)|((DWORD)(BYTE)(ch3)<<24))

/* GUIDs */
#define REFIID const IID *
#define REFGUID const GUID *
#define REFCLSID const IID *
#define DEFINE_GUID(name,l,w1,w2,b1,b2,b3,b4,b5,b6,b7,b8) extern const GUID name
#define DECLSPEC_UUID(x)
#define __CRT_UUID_DECL(type,l,w1,w2,b1,b2,b3,b4,b5,b6,b7,b8)

/* COM interfaces in C layout: each interface is { const <iface>Vtbl *lpVtbl; } */
#define interface struct
#define PURE
#define THIS_ INTERFACE *This,
#define THIS INTERFACE *This
#define STDMETHODCALLTYPE __stdcall
#define STDMETHOD(method) HRESULT (__stdcall *method)
#define STDMETHOD_(type,method) type (__stdcall *method)
#define STDAPI HRESULT __stdcall
#define STDAPI_(type) type __stdcall
#define DECLARE_INTERFACE(iface) typedef struct iface { const struct iface##Vtbl *lpVtbl; } iface; typedef struct iface##Vtbl iface##Vtbl; struct iface##Vtbl
#define DECLARE_INTERFACE_(iface,baseiface) DECLARE_INTERFACE(iface)
#define DECLARE_INTERFACE_IID(iface,iid) DECLARE_INTERFACE(iface)
#define DECLARE_INTERFACE_IID_(iface,baseiface,iid) DECLARE_INTERFACE(iface)
