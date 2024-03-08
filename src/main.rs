use std::collections::HashMap;
use std::str::FromStr;

use tiny_http::{ Server, Response, Header, Method };
use reqwest::header::{ HeaderMap, ACCEPT, CONTENT_TYPE };

mod load;
mod url;
mod utils;

//rfc 1035
//rfc 8484

fn get_header<'a>(find_header: &'a str, headers: &'a [Header]) -> Option<&'a str> {
  for header in headers {
    if header.field.as_str() == find_header {
      return Some(header.value.as_str());
    }
  }
  return None;
}

fn extract_subdomain<'a>(host: &'a str, intranet_host: &'a str) -> &'a str {
  host.get(..host.len() - intranet_host.len() - 1).unwrap()
}

fn is_intranet_subdomain(subdomain: &str, intranet_subdomains: &HashMap<String, String>) -> bool {
  intranet_subdomains.keys().collect::<Vec<&String>>().contains(&&subdomain.to_string())
}

enum QueryError {
  NXDomain,
  NonIntranet,
}

fn do_dns_query(host: String, internal_ip: String, intranet_host: &String, intranet_subdomains: &HashMap<String, String>) -> Result<String, QueryError> {
  if host == "dns.".to_string() + intranet_host {
    //hey, that's us! return our ip address
    Ok(internal_ip)
  } else if host.ends_with(intranet_host) {
    //direct request to intranet site, return ip address of the subdomain
    if let Some(sub_ip) = intranet_subdomains.get(extract_subdomain(&host, &intranet_host)).cloned() {
      Ok(sub_ip)
    } else {
      Err(QueryError::NXDomain)
    }
  } else if is_intranet_subdomain(extract_subdomain(&host, &intranet_host), &intranet_subdomains) {
    //request to intranet tld (that will redirect to intranet subdomain),
    //return our ip address
    Ok(internal_ip)
  } else {
    Err(QueryError::NonIntranet)
  }
}

//start param is a different index to start at (used for pointers)
//not the biggest fan of usize but whatever man
fn extract_host_from_dns_query(dns_query: &[u8], start: Option<usize>) -> Result<String, ()> {
  //13th byte should be start of question, indicate how long the label/zone is in bytes
  let mut length_pos: usize = 12;
  if start.is_some() {
    length_pos = start.unwrap();
  }
  let mut domain_name: Vec<String> = Vec::new();
  let query_len = dns_query.len();
  loop {
    if length_pos >= query_len {
      return Err(());
    }
    let length = dns_query[length_pos];
    if length_pos + usize::from(length) >= dns_query.len() {
      return Err(());
    }
    if length == 0 {
      //the last ., ended
      break;
    }
    let label;
    if length >= 192 {
      //first two bits are 11, is pointer
      let offset: usize = utils::binary_to_u8(&utils::to_binary(length, false)[2..]).into();
      label = extract_host_from_dns_query(dns_query, Some(offset))?;
    } else {
      //ascii
      let label_bytes = &dns_query[(length_pos + 1)..=(length_pos + usize::from(length))];
      label = label_bytes.iter().map(|c_u8| char::from(c_u8.clone()).to_string()).collect::<Vec<String>>().join("");
    }
    domain_name.push(label);
    length_pos = length_pos + usize::from(length) + 1;
  }
  return Ok(domain_name.join("."));
}

fn main() {
  let env = load::load_env();
  println!("{:?}", env);
  
  let INTRANET_HOST = env.get(&"INTRANET_HOST".to_string()).unwrap();
  let INTERNAL_IP = env.get(&"INTERNAL_IP".to_string()).unwrap();
  let PORT = 8443;

  let intranet_subdomains = load::get_intranet_subdomains();

  let client = reqwest::blocking::Client::new();

  let server = Server::http(INTERNAL_IP.to_owned() + ":" + &PORT.to_string()).unwrap();

  for mut request in server.incoming_requests() {
    println!("url: {:?} method {:?} headers: {:?}", request.url(), request.method(), request.headers());
    let method = request.method();
    let url = request.url();
    let path = url::get_path(url);
    let maybe_host = get_header("Host", request.headers());

    if maybe_host.is_some() {
      let host = maybe_host.unwrap();
      //this part is the resolver
      if host == "dns.".to_string() + INTRANET_HOST || host == INTERNAL_IP.to_string() + ":" + &PORT.to_string() {
        //dns request, yay
        //https://www.rfc-editor.org/rfc/rfc8484
        if path == "/dns-query" {
          //this part is the DoH stuff
          //todo: accept: application/dns-message response header
          //todo: make sure content-type is application/dns-message
          if method == &Method::Get || method == &Method::Post {
            let dns_query: Option<Vec<u8>>;
            if method == &Method::Get {
              //read query param
              let queries = url::get_queries(url);
              if let Some(query) = queries.get(&"dns".to_string()) {
                //base64 to bytes
                if let Ok(u8_vec) = utils::b64_url_to_u8_vec(query) {
                  dns_query = Some(u8_vec);
                } else {
                  //error parsing the b64
                  dns_query = None;
                }
              } else {
                dns_query = None;
              }
            } else if method == &Method::Post {
              if let Some("application/dns-message") = get_header("Content-Type", request.headers()) {
                //read post data
                let mut post_data = Vec::new();
                request.as_reader().read_to_end(&mut post_data).unwrap();
                dns_query = Some(post_data);
              } else {
                dns_query = None;
              }
            } else {
              dns_query = None;
            }
            if dns_query.is_some() {
              let dns_query = dns_query.unwrap(); //u8 vec
              //Identification (not needed for DoH, should be 0), 16 bits
              /*
              Flags (total 16 bits)
              QR (query: 0, reply: 1), 1 bit
              OPCODE (standard: 0, inverse: 1, status: 2), 4 bits
              AA (if authorative answer for hostname), 1 bit
              TC (whether message was truncated), 1 bit
              RD (where recursion desired), 1 bit
              RA (in response, whether recursion available), 1 bit
              Z (reserved), 3 bits
              RCODE (response code, NOERROR: 0, FORM(at)ERR: 1, SERVFAIL: 2, NXDOMAIN: 3), 4 bits
              */
              //# of questions, 16 bits
              //# of answers (response), 16 bits
              //# of authority resource records (aka RR), 16 bits
              //# of additional RRs, 16 bits
              //so above is 12 bytes
              /*
              Question section
              Can have many questions, but usually only one (see "# of questions" field)
              Format:
              NAME, variable length
              - NAME is divided into multiple zones/labels (eg: en, wikipedia, org), each zone has 8 bits indicating how many bytes the length is (eg, en is 2 bytes), then the name in ascii, repeated), 0 indicates the NAME is done
              - labels start with 00
              - labels start with length of label
              - NAME or a zone can also be a pointer (16 bits) if first two bits are 11, other 14 bits are an offset
              TYPE (A, AAAA, MX, TXT, etc), 16 bits
              CLASS (probably IN for internet), 16 bits
              */
              /*
              Answer section
              Can have many answer, but usually only one (??) (see "# of answers" field)
              RR Format:
              length of label??? (NAME?)
              NAME, variable length
              TYPE (A, AAAA, MX, TXT, etc), 16 bits
              CLASS, 16 bits
              TTL (time record is valid for), 32 bits
              RDLENGTH (length of RDATA in bytes), 16 bits
              RDATA (additionall RR specific data, see RDLENGTH), variable length
              */
              //Authority section: RRs that point toward authority (NOT RELEVANT for us)
              //Additional space section: RRs with additional information (NOT RELEVANT for us)
              //first do some sanity checks, extract ID (should be 0 but w/e), make sure is query
              //also only accept if one question (pretty sure no one does multiple nowadays anyways)
              //also check length
              //TODO
              //
              //extract the host name
              let query_host_wrapped = extract_host_from_dns_query(&dns_query, None);
              if let Ok(query_host) = query_host_wrapped {
                //now actual dns query stuff, and http response
                let mut header_map = HeaderMap::new();
                header_map.insert(ACCEPT, "application/dns-message".parse().unwrap());
                header_map.insert(CONTENT_TYPE, "application/dns-message".parse().unwrap());
                match do_dns_query(query_host, INTERNAL_IP.clone(), &INTRANET_HOST, &intranet_subdomains) {
                  Ok(ip) => {
                    //construct response
                    //
                  },
                  Err(QueryError::NXDomain) => {
                    //Not found
                    //
                  },
                  Err(QueryError::NonIntranet) => {
                    //regular domain, ens or handshake domain
                    //hnsdns handles all, how nice. No adblock though, like mullvad...
                    //forward query to https://doh.hnsdns.com/dns-query, and return what it returns
                    let res = client.post("https://doh.hnsdns.com/dns-query").body(dns_query).headers(header_map).send().unwrap(); //in the future, throw 500 if fails
                                                                                                     let res_status = res.status().as_u16();
                    let response = Response::from_data(res.bytes().unwrap()).with_status_code(res_status).with_header(Header::from_str("Content-Type: application/dns-message").unwrap()).with_header(Header::from_str("Accept: application/dns-message").unwrap());
                    let _ = request.respond(response);
                  },
                }
              } else {
                //400 bad request, since could not find host in question section of query
                let _ = request.respond(Response::empty(400));
              }
            } else {
              //400 bad request, since missing dns post data or query,
              //or wrong content-type on post
              let _ = request.respond(Response::empty(400));
            }
          } else {
            //405 method not allowed
            let _ = request.respond(Response::empty(405));
          }
        } else {
          //404 not found
          let _ = request.respond(Response::empty(404));
        }
      } else if host.ends_with(INTRANET_HOST) {
        //direct request to intranet site, figure out which one
        //
      } else if is_intranet_subdomain(extract_subdomain(host, INTRANET_HOST), &intranet_subdomains) {
        //request to intranet tld, respond with redirect to intranet site
        //
      } else {
        //421 misdirected, since we dont know wtf this host is. it's not us!
        let _ = request.respond(Response::empty(421));
        /*
        let response = Response::from_string("test");
        let _ = request.respond(response);
        */
      }
    } else {
      //400 bad request, since missing Host header
      let _ = request.respond(Response::empty(400));
    }
  }
}
