using System;
using System.IO;
using System.Threading;
using Unity.Collections;

namespace SaveOptimizerCapture
{
    public static class ReferenceCapture
    {
        private static readonly object Gate = new object();
        private static BinaryWriter writer;
        private static BinaryWriter strideWriter;
        private static Timer timer;
        private static long count;
        private static bool disabled;

        public static void Record(NativeArray<byte> buffer, int offset, int id)
        {
            if (disabled || offset < 0 || offset > buffer.Length - 4)
                return;
            try
            {
                // 展開バッファを識別する候補。Rust側で位置・IDを再照合する。
                ulong hash = Fingerprint(buffer);
                lock (Gate)
                {
                    if (writer == null)
                    {
                        if (!Open())
                            return;
                    }
                    writer.Write(hash);
                    writer.Write(buffer.Length);
                    writer.Write(offset);
                    writer.Write(id);
                    if ((++count & 4095) == 0)
                        writer.Flush();
                }
            }
            catch (Exception error)
            {
                disabled = true;
                Console.Error.WriteLine("SaveOptimizer reference capture stopped: " + error);
            }
        }

        public static void RecordStride(NativeArray<byte> source, NativeArray<byte> child, int end, int stride, ulong before)
        {
            if (disabled || child.Length == 0 || end < child.Length || end > source.Length)
                return;
            try
            {
                ulong sourceHash = Fingerprint(source);
                ulong childHash = Fingerprint(child);
                lock (Gate)
                {
                    if (writer == null && !Open())
                        return;
                    strideWriter.Write(before);
                    strideWriter.Write(source.Length);
                    strideWriter.Write(end - child.Length);
                    strideWriter.Write(childHash);
                    strideWriter.Write(child.Length);
                    strideWriter.Write(stride);
                    strideWriter.Write(sourceHash);
                }
            }
            catch (Exception error)
            {
                disabled = true;
                Console.Error.WriteLine("SaveOptimizer stride capture stopped: " + error);
            }
        }

        public static ulong Fingerprint(NativeArray<byte> buffer)
        {
            ulong hash = 14695981039346656037UL;
            int length = buffer.Length;
            int samples = Math.Min(length, 32);
            for (int i = 0; i < samples; i++)
            {
                int at = (int)((long)i * (length - 1) / Math.Max(1, samples - 1));
                hash = unchecked((hash ^ buffer[at]) * 1099511628211UL);
            }
            return hash;
        }

        private static bool Open()
        {
            string path = Environment.GetEnvironmentVariable("CS2_REFERENCE_CAPTURE");
            if (string.IsNullOrEmpty(path))
            {
                disabled = true;
                return false;
            }
            writer = NewWriter(path, 0x31524353);
            strideWriter = NewWriter(path + ".stride", 0x32534353);
            timer = new Timer(delegate { Flush(null, null); }, null, 1000, 1000);
            AppDomain.CurrentDomain.ProcessExit += Flush;
            return true;
        }

        private static BinaryWriter NewWriter(string path, int magic)
        {
            BinaryWriter output = new BinaryWriter(new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.Read, 65536));
            output.Write(magic);
            return output;
        }

        private static void Flush(object sender, EventArgs args)
        {
            lock (Gate)
            {
                if (writer != null)
                    writer.Flush();
                if (strideWriter != null)
                    strideWriter.Flush();
                GC.KeepAlive(timer);
            }
        }
    }
}
