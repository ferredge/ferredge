/* QEMU mps2-an385 (Cortex-M3): code in ZBT SSRAM1, data in on-board SRAM. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 4M
  RAM   : ORIGIN = 0x20000000, LENGTH = 4M
}
