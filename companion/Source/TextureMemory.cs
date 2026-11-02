using System;
using System.Collections.Generic;
using Verse;
#if NETFRAMEWORK
using UnityEngine;
#endif

namespace RimWorldModManagerCompanion
{
    /// <summary>
    /// Video memory of the loaded textures, per mod. 1.6 only: on 1.7 the game uploads and
    /// evicts textures itself, so there is no fixed amount to count.
    /// </summary>
    internal static class TextureMemory
    {
#if NETFRAMEWORK
        public static string Json(List<ModContentPack> mods)
        {
            var json = new JsonWriter();
            json.BeginObject();
            long allBytes = 0;
            int allCount = 0;
            foreach (Texture2D t in Resources.FindObjectsOfTypeAll<Texture2D>())
            {
                if (t == null) continue;
                allBytes += Bytes(t);
                allCount++;
            }
            json.Field("all_bytes", allBytes);
            json.Field("all_count", allCount);
            json.Field("engine_bytes", (long)Texture.totalTextureMemory);
            json.Field("vram_mb", SystemInfo.graphicsMemorySize);
            json.Name("mods");
            json.BeginArray();
            for (int i = 0; i < mods.Count; i++)
            {
                Dictionary<string, Texture2D> textures = mods[i].GetContentHolder<Texture2D>()?.contentList;
                if (textures == null) continue;
                long bytes = 0, uncompressedBytes = 0, readableBytes = 0;
                int count = 0, uncompressed = 0;
                foreach (Texture2D t in textures.Values)
                {
                    if (t == null) continue;
                    long b = Bytes(t);
                    bytes += b;
                    count++;
                    if (BlockBytes(t.format) == 0)
                    {
                        uncompressed++;
                        uncompressedBytes += b;
                    }
                    // A readable texture keeps a copy in RAM too.
                    if (t.isReadable) readableBytes += b;
                }
                if (count == 0) continue;
                json.BeginObject();
                json.Field("mod", i);
                json.Field("count", count);
                json.Field("bytes", bytes);
                json.Field("uncompressed", uncompressed);
                json.Field("uncompressed_bytes", uncompressedBytes);
                json.Field("readable_bytes", readableBytes);
                json.EndObject();
            }
            json.EndArray();
            json.EndObject();
            return json.ToString();
        }

        /// <summary>Size on the graphics card, mipmaps included.</summary>
        private static long Bytes(Texture2D t)
        {
            int block = BlockBytes(t.format);
            int pixel = PixelBytes(t.format);
            int levels = Math.Max(1, t.mipmapCount);
            long total = 0;
            for (int level = 0; level < levels; level++)
            {
                long w = Math.Max(1, t.width >> level);
                long h = Math.Max(1, t.height >> level);
                total += block > 0 ? ((w + 3) / 4) * ((h + 3) / 4) * block : w * h * pixel;
            }
            return total;
        }

        /// <summary>Bytes per 4x4 block of a compressed format; 0 for an uncompressed one.</summary>
        private static int BlockBytes(TextureFormat format)
        {
            switch (format)
            {
                case TextureFormat.DXT1:
                case TextureFormat.DXT1Crunched:
                case TextureFormat.BC4:
                    return 8;
                case TextureFormat.DXT5:
                case TextureFormat.DXT5Crunched:
                case TextureFormat.BC5:
                case TextureFormat.BC6H:
                case TextureFormat.BC7:
                    return 16;
                default:
                    return 0;
            }
        }

        /// <summary>Bytes per pixel of an uncompressed format (24-bit RGB takes 4 on the card).</summary>
        private static int PixelBytes(TextureFormat format)
        {
            switch (format)
            {
                case TextureFormat.Alpha8:
                case TextureFormat.R8:
                    return 1;
                case TextureFormat.ARGB4444:
                case TextureFormat.RGBA4444:
                case TextureFormat.RGB565:
                case TextureFormat.R16:
                case TextureFormat.RHalf:
                case TextureFormat.RG16:
                    return 2;
                case TextureFormat.RGHalf:
                case TextureFormat.RFloat:
                    return 4;
                case TextureFormat.RGBAHalf:
                case TextureFormat.RGFloat:
                    return 8;
                case TextureFormat.RGBAFloat:
                    return 16;
                default:
                    return 4;
            }
        }
#else
        public static string Json(List<ModContentPack> mods) => null;
#endif
    }
}
