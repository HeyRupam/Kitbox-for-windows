; Uninstalling Kitbox takes its right-click menu with it.
;
; The menu lives under HKEY_CURRENT_USER, so it survives an uninstall unless
; something removes it — and a context menu pointing at a program that is no
; longer installed is one of the nastier things to leave on someone's machine.
;
; These keys mirror `shell.rs`: one per file type the menu is offered on. If
; that list changes, this changes with it.

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.jpg\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.jpeg\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.png\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.webp\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.bmp\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.tif\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.tiff\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.heic\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.gif\shell\Kitbox"
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.pdf\shell\Kitbox"
!macroend
