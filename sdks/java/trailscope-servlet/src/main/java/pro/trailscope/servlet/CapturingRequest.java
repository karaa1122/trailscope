package pro.trailscope.servlet;

import jakarta.servlet.AsyncContext;
import jakarta.servlet.ReadListener;
import jakarta.servlet.ServletContext;
import jakarta.servlet.ServletException;
import jakarta.servlet.ServletInputStream;
import jakarta.servlet.ServletResponse;
import jakarta.servlet.http.HttpServletRequest;
import jakarta.servlet.http.HttpServletRequestWrapper;
import jakarta.servlet.http.Part;
import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.UnsupportedEncodingException;
import java.net.URLDecoder;
import java.net.URLEncoder;
import java.nio.charset.Charset;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Enumeration;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/**
 * Copies the first {@code maxBodyBytes + 1} bytes of the request body as the application reads
 * it. Nothing is read on the application's behalf, and large bodies are never buffered whole.
 */
final class CapturingRequest extends HttpServletRequestWrapper {
  private final BoundedBuffer body;
  private final boolean capture;
  private ServletResponse response;
  private ServletInputStream stream;
  private BufferedReader reader;
  private boolean parametersRead;
  private boolean partsRead;

  CapturingRequest(HttpServletRequest request, int cap) {
    super(request);
    this.capture = cap > 0;
    this.body = new BoundedBuffer(cap);
  }

  void setResponse(ServletResponse response) {
    this.response = response;
  }

  long bytesRead() {
    return body.total();
  }

  @Override
  public ServletInputStream getInputStream() throws IOException {
    if (!capture) {
      return super.getInputStream();
    }
    if (stream == null) {
      stream = new TeeInputStream(super.getInputStream(), body);
    }
    return stream;
  }

  @Override
  public BufferedReader getReader() throws IOException {
    if (!capture) {
      return super.getReader();
    }
    if (reader == null) {
      reader = new BufferedReader(new InputStreamReader(getInputStream(), readerCharset()));
    }
    return reader;
  }

  private Charset readerCharset() throws UnsupportedEncodingException {
    String name = getCharacterEncoding();
    if (name == null) {
      ServletContext sc = getServletContext();
      name = sc != null ? sc.getRequestCharacterEncoding() : null;
    }
    if (name == null) {
      return StandardCharsets.ISO_8859_1;
    }
    try {
      return Charset.forName(name);
    } catch (IllegalArgumentException e) {
      throw new UnsupportedEncodingException(name);
    }
  }

  // Keeps our wrappers in place for async dispatches, so the response body is still copied.
  @Override
  public AsyncContext startAsync() {
    return response != null ? super.startAsync(this, response) : super.startAsync();
  }

  @Override
  public String getParameter(String name) {
    parametersRead = true;
    return super.getParameter(name);
  }

  @Override
  public Map<String, String[]> getParameterMap() {
    parametersRead = true;
    return super.getParameterMap();
  }

  @Override
  public Enumeration<String> getParameterNames() {
    parametersRead = true;
    return super.getParameterNames();
  }

  @Override
  public String[] getParameterValues(String name) {
    parametersRead = true;
    return super.getParameterValues(name);
  }

  @Override
  public Collection<Part> getParts() throws IOException, ServletException {
    partsRead = true;
    return super.getParts();
  }

  @Override
  public Part getPart(String name) throws IOException, ServletException {
    partsRead = true;
    return super.getPart(name);
  }

  /**
   * The body as far as the application read it: raw bytes, the form fields the container parsed
   * (when the application read parameters instead of the stream), or a multipart form's text
   * fields with files listed by name, size and type. Null when nothing was read.
   */
  Object capturedBody(int limit) {
    Object raw = body.content();
    if (raw != null) {
      return raw;
    }
    String ct = getContentType();
    String type = ct == null ? "" : ct.toLowerCase(Locale.ROOT);
    try {
      if (partsRead && type.startsWith("multipart/")) {
        return multipartFields(limit);
      }
      if (parametersRead && type.startsWith("application/x-www-form-urlencoded")) {
        return formBody(limit);
      }
    } catch (Exception e) {
      return null;
    }
    return null;
  }

  private Map<String, Object> multipartFields(int limit) throws IOException, ServletException {
    Map<String, Object> out = new LinkedHashMap<>();
    int size = 0;
    Collection<Part> parts =
        getRequest() instanceof HttpServletRequest r ? r.getParts() : List.<Part>of();
    for (Part part : parts) {
      String name = part.getName();
      Object value;
      if (part.getSubmittedFileName() == null) {
        String[] values = getRequest().getParameterValues(name);
        if (values == null) {
          value = "[field, " + part.getSize() + " bytes]";
        } else {
          value = values.length == 1 ? values[0] : List.of(values);
        }
      } else {
        value =
            "[file "
                + part.getSubmittedFileName()
                + ", "
                + part.getSize()
                + " bytes, "
                + part.getContentType()
                + "]";
      }
      Object prev = out.get(name);
      if (prev == null) {
        out.put(name, value);
      } else if (prev instanceof List<?> list) {
        @SuppressWarnings("unchecked")
        List<Object> l = (List<Object>) list;
        l.add(value);
      } else {
        out.put(name, new ArrayList<>(List.of(prev, value)));
      }
      size += name.length() + String.valueOf(value).length();
      if (size > limit) {
        break;
      }
    }
    return out.isEmpty() ? null : out;
  }

  private String formBody(int limit) {
    Set<String> fromQuery = new HashSet<>();
    String query = getQueryString();
    if (query != null) {
      for (String pair : query.split("&")) {
        int eq = pair.indexOf('=');
        String k = eq < 0 ? pair : pair.substring(0, eq);
        try {
          fromQuery.add(URLDecoder.decode(k, StandardCharsets.UTF_8));
        } catch (IllegalArgumentException e) {
          fromQuery.add(k);
        }
      }
    }
    StringBuilder sb = new StringBuilder();
    for (Map.Entry<String, String[]> e : getRequest().getParameterMap().entrySet()) {
      if (fromQuery.contains(e.getKey())) {
        continue;
      }
      for (String v : e.getValue()) {
        if (sb.length() > 0) {
          sb.append('&');
        }
        sb.append(URLEncoder.encode(e.getKey(), StandardCharsets.UTF_8))
            .append('=')
            .append(URLEncoder.encode(v, StandardCharsets.UTF_8));
        if (sb.length() > limit) {
          return sb.toString();
        }
      }
    }
    return sb.length() == 0 ? null : sb.toString();
  }

  private static final class TeeInputStream extends ServletInputStream {
    private final ServletInputStream in;
    private final BoundedBuffer body;

    TeeInputStream(ServletInputStream in, BoundedBuffer body) {
      this.in = in;
      this.body = body;
    }

    @Override
    public int read() throws IOException {
      int b = in.read();
      if (b >= 0) {
        body.write(b);
      }
      return b;
    }

    @Override
    public int read(byte[] b, int off, int len) throws IOException {
      int n = in.read(b, off, len);
      if (n > 0) {
        body.write(b, off, n);
      }
      return n;
    }

    @Override
    public int readLine(byte[] b, int off, int len) throws IOException {
      int n = in.readLine(b, off, len);
      if (n > 0) {
        body.write(b, off, n);
      }
      return n;
    }

    @Override
    public int available() throws IOException {
      return in.available();
    }

    @Override
    public void close() throws IOException {
      in.close();
    }

    @Override
    public boolean isFinished() {
      return in.isFinished();
    }

    @Override
    public boolean isReady() {
      return in.isReady();
    }

    @Override
    public void setReadListener(ReadListener readListener) {
      in.setReadListener(readListener);
    }
  }
}
