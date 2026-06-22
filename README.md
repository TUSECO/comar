# Gönüllü

The Gönüllü application is designed to compile packages for the LupuS distribution. Using Docker images, the application converts packages waiting in the build system queue into pisi packages and sends them to the build system.

You do not need to use LupuS to use this application. You can use any Linux distribution that supports Docker.

## USE:

To download the application to your system:

  	   git clone https://github.com/TeknoAnka/GONULLU.git
  	   cd gonullu
  	   sudo python3 setup.py install
  	   
OR

  	   sudo pip3 install git+https://github.com/TeknoAnka/GONULLU.git
  	   
To update the application:

  	   sudo pip3 install git+https://github.com/TeknoAnka/GONULLU.git --upgrade

To see the parameters that can be given to the application:

  	   gonullu -k

OR
       
       gonullu --kullanim

If used without specifying any parameters, it is configured to use 1 CPU and 50% of the memory.

## Parameters:

* -k OR --kullanim:
Contains information about the use of the software.

* -m OR --memory
The limit of physical RAM that can be used by Docker.

* -c OR --cpu
Number of CPUs to be allocated for use by Docker

* -e OR --email:
Mail address
