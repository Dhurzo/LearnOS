; OS Kernel Boot Sector Assembly
; 
; This is a boot sector implementation that serves as a fallback boot method
; when the main PVH (Para-Virtualized Hypervisor) kernel fails to work.
; It uses BIOS interrupts to display "Hello World!" on the screen and demonstrates
; fundamental boot sector programming concepts.
; 
; Boot Sector Format:
; - Total size: 512 bytes (1 sector)
; - Signature: 0xAA55 at offset 510-511
; - Code: Starts at offset 0
; 
; Boot Process:
; 1. BIOS loads this boot sector at address 0x7C00 (31KB)
; 2. BIOS jumps to 0x7C00 to execute the code
; 3. Boot sector sets up video mode and displays message
; 4. Boot sector enters infinite loop to prevent reboot
; 
; For more information about boot sector programming:
; - [OSDev Wiki - Boot Sector](https://wiki.osdev.org/Boot_Sector)
; - [BIOS Interrupt 0x10](https://www.osdever.net/bb/viewtopic.php?f=1&t=2173)

bits 16                         ; Use 16-bit instructions (real mode)
org 0x7C00                       ; Origin address - where this code will be loaded

start:
    ; Switch to VGA text mode 80x25 (mode 3)
    ; This provides a clean 80-column, 25-row text display
    mov ax, 0x0003              ; AH=0x00 (set video mode), AL=0x03 (80x25 color text)
    int 0x10                     ; Video services interrupt
    
    ; Set cursor position to top-left corner (row 0, column 0)
    ; This moves the cursor to the beginning of the screen for clean output
    mov ah, 0x02                 ; Function 0x02: Set cursor position
    mov bh, 0x00                 ; Page number (0 for primary display)
    mov dh, 0x00                 ; Row position (0 = top)
    mov dl, 0x00                 ; Column position (0 = left)
    int 0x10                     ; Video services interrupt
    
    ; Print "Hello World!" using BIOS teletype output
    ; This function prints characters directly to the screen with automatic scrolling
    mov si, message              ; SI pointer to the message string
    mov cx, message_len          ; CX = length of message (number of characters to print)
    mov ah, 0x0E                 ; Function 0x0E: Teletype output
    mov bh, 0x00                 ; Page number (0 for primary display)
    mov bl, 0x0F                 ; Color attribute: White text on black background
                                    ; BL format: 0xF = 00001111 (bright white text, black background)
    
print_loop:
    lodsb                        ; Load byte from [SI] and increment SI
    int 0x10                     ; Video services interrupt (prints character in AL)
    loop print_loop              ; Decrement CX and jump if CX != 0
    
    ; Enter infinite loop to prevent reboot
    ; This keeps the system running indefinitely. In a real kernel,
    ; this would be replaced with proper kernel initialization.
    jmp $                        ; Jump to self - creates an infinite loop

; Data section - message and its length
message: db 'Hello World!'        ; Null-terminated string (ASCII characters)
message_len equ $ - message       ; Calculate length: current position - message start
                                    ; $ = current address, $$ = section start
                                    ; This gives us the length at compile time

; Fill with zeros to make 512 bytes
times 510-($-$$) db 0

; Boot sector signature (0xAA55 at offset 510)
dw 0xAA55
