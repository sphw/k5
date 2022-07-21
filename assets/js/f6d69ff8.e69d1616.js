"use strict";(self.webpackChunkdocs=self.webpackChunkdocs||[]).push([[463],{3905:(e,t,r)=>{r.d(t,{Zo:()=>c,kt:()=>h});var o=r(7294);function a(e,t,r){return t in e?Object.defineProperty(e,t,{value:r,enumerable:!0,configurable:!0,writable:!0}):e[t]=r,e}function n(e,t){var r=Object.keys(e);if(Object.getOwnPropertySymbols){var o=Object.getOwnPropertySymbols(e);t&&(o=o.filter((function(t){return Object.getOwnPropertyDescriptor(e,t).enumerable}))),r.push.apply(r,o)}return r}function l(e){for(var t=1;t<arguments.length;t++){var r=null!=arguments[t]?arguments[t]:{};t%2?n(Object(r),!0).forEach((function(t){a(e,t,r[t])})):Object.getOwnPropertyDescriptors?Object.defineProperties(e,Object.getOwnPropertyDescriptors(r)):n(Object(r)).forEach((function(t){Object.defineProperty(e,t,Object.getOwnPropertyDescriptor(r,t))}))}return e}function s(e,t){if(null==e)return{};var r,o,a=function(e,t){if(null==e)return{};var r,o,a={},n=Object.keys(e);for(o=0;o<n.length;o++)r=n[o],t.indexOf(r)>=0||(a[r]=e[r]);return a}(e,t);if(Object.getOwnPropertySymbols){var n=Object.getOwnPropertySymbols(e);for(o=0;o<n.length;o++)r=n[o],t.indexOf(r)>=0||Object.prototype.propertyIsEnumerable.call(e,r)&&(a[r]=e[r])}return a}var i=o.createContext({}),p=function(e){var t=o.useContext(i),r=t;return e&&(r="function"==typeof e?e(t):l(l({},t),e)),r},c=function(e){var t=p(e.components);return o.createElement(i.Provider,{value:t},e.children)},u={inlineCode:"code",wrapper:function(e){var t=e.children;return o.createElement(o.Fragment,{},t)}},d=o.forwardRef((function(e,t){var r=e.components,a=e.mdxType,n=e.originalType,i=e.parentName,c=s(e,["components","mdxType","originalType","parentName"]),d=p(r),h=a,m=d["".concat(i,".").concat(h)]||d[h]||u[h]||n;return r?o.createElement(m,l(l({ref:t},c),{},{components:r})):o.createElement(m,l({ref:t},c))}));function h(e,t){var r=arguments,a=t&&t.mdxType;if("string"==typeof e||a){var n=r.length,l=new Array(n);l[0]=d;var s={};for(var i in t)hasOwnProperty.call(t,i)&&(s[i]=t[i]);s.originalType=e,s.mdxType="string"==typeof e?e:a,l[1]=s;for(var p=2;p<n;p++)l[p]=r[p];return o.createElement.apply(null,l)}return o.createElement.apply(null,r)}d.displayName="MDXCreateElement"},2264:(e,t,r)=>{r.r(t),r.d(t,{assets:()=>i,contentTitle:()=>l,default:()=>u,frontMatter:()=>n,metadata:()=>s,toc:()=>p});var o=r(7462),a=(r(7294),r(3905));const n={sidebar_position:1,slug:"/"},l="K5 Microkernel",s={unversionedId:"intro",id:"intro",title:"K5 Microkernel",description:"K5 is a small microkernel closely related to the L4 family. It's niche is in embedded systems that need some element of security and/or safety guarenetee. Think industrial equipment, or secure elements. It currently runs on ARMv8m microcontrollers, and is being tested on the STM32L56. Eventually we plan to port K5 to other the other Cortex-M versions and RISC-V.",source:"@site/../docs/intro.md",sourceDirName:".",slug:"/",permalink:"/",draft:!1,tags:[],version:"current",sidebarPosition:1,frontMatter:{sidebar_position:1,slug:"/"},sidebar:"tutorialSidebar"},i={},p=[{value:"Getting Started",id:"getting-started",level:2},{value:"Install CLI",id:"install-cli",level:3},{value:"STM32L5 Example",id:"stm32l5-example",level:3},{value:"Goals",id:"goals",level:2},{value:"Small readable code-base.",id:"small-readable-code-base",level:4},{value:"First-class developer experience",id:"first-class-developer-experience",level:4},{value:"Employ formal verification methods and other static analysis tools everywhere possible.",id:"employ-formal-verification-methods-and-other-static-analysis-tools-everywhere-possible",level:4},{value:"Strong task isolation",id:"strong-task-isolation",level:4},{value:"Native enclave support",id:"native-enclave-support",level:4},{value:"Non-Goals",id:"non-goals",level:2},{value:"General purpose OS",id:"general-purpose-os",level:4},{value:"Plug and play driver support",id:"plug-and-play-driver-support",level:4}],c={toc:p};function u(e){let{components:t,...r}=e;return(0,a.kt)("wrapper",(0,o.Z)({},c,r,{components:t,mdxType:"MDXLayout"}),(0,a.kt)("h1",{id:"k5-microkernel"},"K5 Microkernel"),(0,a.kt)("p",null,"K5 is a small microkernel closely related to the L4 family. It's niche is in embedded systems that need some element of security and/or safety guarenetee. Think industrial equipment, or secure elements. It currently runs on ARMv8m microcontrollers, and is being tested on the STM32L56. Eventually we plan to port K5 to other the other Cortex-M versions and RISC-V. "),(0,a.kt)("h2",{id:"getting-started"},"Getting Started"),(0,a.kt)("h3",{id:"install-cli"},"Install CLI"),(0,a.kt)("p",null,"The easiest way is by running "),(0,a.kt)("pre",null,(0,a.kt)("code",{parentName:"pre",className:"language-sh"},"cargo install --git https://github.com/sphw/k5.git --bins k5\n")),(0,a.kt)("h3",{id:"stm32l5-example"},"STM32L5 Example"),(0,a.kt)("p",null,"Then go to ",(0,a.kt)("inlineCode",{parentName:"p"},"./examples/stm32l5")," and run ",(0,a.kt)("inlineCode",{parentName:"p"},"k5 logs"),". If everything goes right you should see something like below."),(0,a.kt)("p",null,(0,a.kt)("a",{parentName:"p",href:"https://asciinema.org/a/509730"},(0,a.kt)("img",{parentName:"a",src:"https://asciinema.org/a/509730.svg",alt:"asciicast"}))),(0,a.kt)("h2",{id:"goals"},"Goals"),(0,a.kt)("h4",{id:"small-readable-code-base"},"Small readable code-base."),(0,a.kt)("p",null,"A primary goal of this project is to create an approchable microkernel. The more people can read and fully understand the code in K5, the less bugs there will be."),(0,a.kt)("h4",{id:"first-class-developer-experience"},"First-class developer experience"),(0,a.kt)("p",null,"Embedded development is quite a mixed bag of tooling, some good, some terrible. K5's goal is to make every supported platform easy to work with. Thankfully the embedded Rust community has made this a lot easier with tools like ",(0,a.kt)("a",{parentName:"p",href:"https://github.com/probe-rs/probe-rs"},"probe-rs")," and ",(0,a.kt)("a",{parentName:"p",href:"https://github.com/knurling-rs/defmt"},"defmt"),". K5 should also have best-in-class userspace libraries, to make it easy to develop new applications."),(0,a.kt)("h4",{id:"employ-formal-verification-methods-and-other-static-analysis-tools-everywhere-possible"},"Employ formal verification methods and other static analysis tools everywhere possible."),(0,a.kt)("p",null,"Software engineering has long been the wild-west of the engineering world. There have been many attempts to improve this state-of-affairs, but they are often so cumbersome that they quickly become abandoned. Rust solves part of this problem through its approach to memory-safety, and there are other promising tools in the Rust eco-system that may allow us to verify large parts of the kernel. seL4 has led the way by formally verifying the entire kernel. In the short-term we plan to verify parts of K5's scheduler using ",(0,a.kt)("a",{parentName:"p",href:"https://github.com/model-checking/kani"},"Kani"),"."),(0,a.kt)("h4",{id:"strong-task-isolation"},"Strong task isolation"),(0,a.kt)("p",null,"Much like seL4, K5 utilizies a capability based system for security. One of K5's goals is to provide strong isolation between tasks, and to ensure that communication only occurs through proper channels. This helps limit the blast-radius of security vulnerabilities."),(0,a.kt)("h4",{id:"native-enclave-support"},"Native enclave support"),(0,a.kt)("p",null,"In recent years enclave support has been addede to a whole variety of process. In particular TrustZone-M and PMP are becoming very common on microcontrollers. Current RTOSes leave it up to the user to figure out enclaves on their own, or they are told to use ARM TF-M (which is difficult to use and incomplete). K5 will provide enclave support for both RISC-V and ARMv8m, and make it a first class citizen on the OS. "),(0,a.kt)("h2",{id:"non-goals"},"Non-Goals"),(0,a.kt)("h4",{id:"general-purpose-os"},"General purpose OS"),(0,a.kt)("p",null,"K5 is not going to be Linux, Darwin, or even Fuschia / Zychron. The goal is to make a kernel for high-security embedded applications, not your laptop. Thankfully, that means we don't have to worry about a whole-host of issues that most OSes worry about."),(0,a.kt)("h4",{id:"plug-and-play-driver-support"},"Plug and play driver support"),(0,a.kt)("p",null,"Have you ever used Zephyr? Its kinda crazy how you can almost seemlessly port code from one board to another with little effort. But have you ever tried to use Zephyr in a non-standard way, yikes. That is NOT K5's goal. We want a healthy eco-system of drivers, hopefully with standard-ized interfaces for common operations, but code-compatibility between unrelated devices is not the end-goal."))}u.isMDXComponent=!0}}]);