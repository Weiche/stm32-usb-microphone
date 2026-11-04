/* STM32F407VG: 1 MiB Flash, 112 KiB SRAM1 + 16 KiB SRAM2.
 * Deliberately do NOT expose 64 KiB CCM (0x10000000): DMA1/2 cannot access it.
 * All .data/.bss, StaticCell storage, executor task frames and stack use SRAM.
 */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
