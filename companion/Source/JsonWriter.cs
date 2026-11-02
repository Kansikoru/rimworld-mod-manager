using System.Globalization;
using System.Text;

namespace RimWorldModManagerCompanion
{
    /// <summary>A minimal JSON writer: the game has no JSON library on 1.6.</summary>
    internal sealed class JsonWriter
    {
        private readonly StringBuilder sb = new StringBuilder();
        private bool needComma;

        public void BeginObject() { Comma(); sb.Append('{'); needComma = false; }
        public void EndObject() { sb.Append('}'); needComma = true; }
        public void BeginArray() { Comma(); sb.Append('['); needComma = false; }
        public void EndArray() { sb.Append(']'); needComma = true; }

        public void Name(string name)
        {
            Comma();
            Quote(name);
            sb.Append(':');
            needComma = false;
        }

        public void Value(string value)
        {
            Comma();
            if (value == null) sb.Append("null"); else Quote(value);
            needComma = true;
        }

        public void Value(long value)
        {
            Comma();
            sb.Append(value.ToString(CultureInfo.InvariantCulture));
            needComma = true;
        }

        public void Value(double value)
        {
            Comma();
            if (double.IsNaN(value) || double.IsInfinity(value)) sb.Append("null");
            else sb.Append(value.ToString("0.###", CultureInfo.InvariantCulture));
            needComma = true;
        }

        public void Value(bool value)
        {
            Comma();
            sb.Append(value ? "true" : "false");
            needComma = true;
        }

        /// <summary>Already written JSON (a whole value), or null.</summary>
        public void Raw(string json)
        {
            Comma();
            sb.Append(json ?? "null");
            needComma = true;
        }

        public void Field(string name, string value) { Name(name); Value(value); }
        public void Field(string name, long value) { Name(name); Value(value); }
        public void Field(string name, double value) { Name(name); Value(value); }
        public void Field(string name, bool value) { Name(name); Value(value); }

        public override string ToString() => sb.ToString();

        private void Comma()
        {
            if (needComma) sb.Append(',');
        }

        private void Quote(string s)
        {
            sb.Append('"');
            foreach (char c in s)
            {
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (c < 0x20) sb.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                        else sb.Append(c);
                        break;
                }
            }
            sb.Append('"');
        }
    }
}
