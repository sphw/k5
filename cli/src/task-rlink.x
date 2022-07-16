/* Linker script for a relocatable task
 * This should be used with the `-r` linker flag */
ENTRY(_start);

SECTIONS
{
  /* ### .text */
  .text : {
    _stext = .;
    *(.text.start*); /* try and pull start symbol to beginning */
    *(.text .text.*);
    . = ALIGN(4);
    __etext = .;
  }

  /* ### .rodata */
  .rodata : ALIGN(4)
  {
    *(.rodata .rodata.*);

    /* 4-byte align the end (VMA) of this section.
       This is required by LLD to ensure the LMA of the following .data
       section will have the correct alignment. */
    . = ALIGN(4);
    __erodata = .;
  }

  /*
   * Sections in RAM
   *
   * NOTE: the userlib runtime assumes that these sections
   * are 4-byte aligned and padded to 4-byte boundaries.
   */
  .data : ALIGN(4) {
    . = ALIGN(4);
    __sdata = .;
    *(.data .data.*);
    . = ALIGN(4); /* 4-byte align the end (VMA) of this section */
    __edata = .;
  }

  .bss (NOLOAD) : ALIGN(4) {
    . = ALIGN(4);
    __sbss = .;
    *(.bss .bss.*);
    . = ALIGN(4); /* 4-byte align the end (VMA) of this section */
    __ebss = .;
  }

  .uninit (NOLOAD) : ALIGN(4) {
    . = ALIGN(4);
    *(.uninit .uninit.*);
    . = ALIGN(4);
    /* Place the heap right after `.uninit` */
    __sheap = .;
  }

  /* ## .got */
  /* Dynamic relocations are unsupported. This section is only used to detect relocatable code in
     the input files and raise an error if relocatable code is found */
  .got (NOLOAD) : {
    KEEP(*(.got .got.*));
  }

  /* ## Discarded sections */
  /DISCARD/ :
  {
    /* Unused exception related info that only wastes space */
    *(.ARM.exidx);
    *(.ARM.exidx.*);
    *(.ARM.extab.*);
  }
}
